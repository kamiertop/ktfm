use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: u64 = 128 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
const FORMATS: &str = concat!(
    "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,asf,flv,mpeg,mpegts,",
    "mp3,flac,ogg,wav,aac,aiff,jpeg_pipe,png_pipe,webp_pipe,gif,gif_pipe,bmp_pipe,tiff_pipe,",
    // jpg/jpeg 文件按扩展名被 image2 解复用器认领，jpeg_pipe 只在管道输入时才会被探测。
    "image2"
);

/// 从本地媒体提取首个画面或内嵌封面，输出尺寸受限的 PNG。
pub fn thumbnail(ffmpeg: &Path, path: &Path) -> Option<Vec<u8>> {
    if !std::fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    // 使用显式文件协议，避免把文件名解析成 URL。
    let mut input = OsString::from("file:");
    input.push(path);
    let mut child = Command::new(ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-filter_threads",
            "1",
            "-threads",
            "1",
            "-protocol_whitelist",
            "file",
            "-format_whitelist",
            FORMATS,
            "-i",
        ])
        .arg(input)
        .args([
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-vf",
            "scale=128:128:force_original_aspect_ratio=decrease",
            "-threads",
            "1",
            "-c:v",
            "png",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take().expect("已设置缩略图输出管道");
    thread::scope(|scope| {
        // 同时读取输出，避免子进程被已满的管道阻塞。
        let output = thread::Builder::new().spawn_scoped(scope, move || {
            let mut bytes = Vec::new();
            stdout.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes).ok()?;
            (!bytes.is_empty() && bytes.len() as u64 <= OUTPUT_LIMIT).then_some(bytes)
        });
        if let Ok(output) = output {
            let deadline = Instant::now() + TIMEOUT;
            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        return output.join().ok().flatten().filter(|_| status.success());
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(15)),
                    Err(_) => break,
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        None
    })
}
