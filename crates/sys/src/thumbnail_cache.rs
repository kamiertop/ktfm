//! freedesktop 缩略图共享缓存（`~/.cache/thumbnails`）的读写。
//!
//! 只写自己生成的条目（`normal/` 桶与 `fail/ktfm/` 失败标记），读取时按规范
//! 校验元数据，因此 Nautilus 等其他应用生成的缩略图可直接复用，反之亦然。
//! 清理与用量统计只针对 `Software` 标记为本应用的文件，不动其他应用的数据。

use std::fs;
use std::io::{self, Cursor, Read};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use url::Url;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// 从小到大的尺寸档位，与规范保持一致。
const BUCKETS: [&str; 4] = ["normal", "large", "x-large", "xx-large"];
const NORMAL_PX: u32 = 128;
const MAX_CACHE_PX: u32 = 1024;
const MAX_CACHE_FILE: u64 = 8 * 1024 * 1024;
const SOFTWARE: &str = "ktfm";

/// 查询共享缓存的结果。
pub enum LoadResult {
    /// 命中有效缩略图。
    Thumbnail(Vec<u8>),
    /// 命中本应用写入的失败标记，文件未变期间不必重试。
    Failed,
    /// 没有任何条目。
    Miss,
}

#[derive(Clone)]
pub struct ThumbnailCache {
    root: PathBuf,
}

pub struct SourceThumbnail {
    path: PathBuf,
    uri: Url,
    filename: String,
    modified: SystemTime,
    size: u64,
}

impl ThumbnailCache {
    pub fn open() -> Option<Self> {
        let root = directories::BaseDirs::new()?.cache_dir().join("thumbnails");
        let cache = Self { root };
        cache.remove_legacy_private();
        Some(cache)
    }

    /// 旧版本把缩略图写在 `~/.cache/ktfm/thumbnails`，迁移后删除避免遗留。
    fn remove_legacy_private(&self) {
        let Some(sibling) = self.root.parent() else {
            return;
        };
        let legacy = sibling.join("ktfm").join("thumbnails");
        if fs::remove_dir_all(&legacy).is_ok() {
            // 目录已空时顺带移除父目录，非空则保留其他数据。
            let _ = fs::remove_dir(legacy.parent().expect("旧缓存目录应有父目录"));
        }
    }

    pub fn source(&self, path: &Path) -> Option<SourceThumbnail> {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        self.source_with_metadata(path, metadata.modified().ok()?, metadata.len())
    }

    pub fn source_with_metadata(
        &self,
        path: &Path,
        modified: SystemTime,
        size: u64,
    ) -> Option<SourceThumbnail> {
        // 拒绝为缓存目录内的文件生成缩略图，避免递归。
        if path.starts_with(&self.root) {
            return None;
        }
        let uri = Url::from_file_path(path).ok()?;
        let filename = format!("{:x}.png", Md5::digest(uri.as_str().as_bytes()));
        Some(SourceThumbnail {
            path: path.to_owned(),
            uri,
            filename,
            modified,
            size,
        })
    }

    pub fn load(&self, source: &SourceThumbnail, device_size: u16) -> LoadResult {
        if self.load_failure(source) {
            return LoadResult::Failed;
        }
        let start = BUCKETS
            .iter()
            .enumerate()
            .position(|(index, _)| u32::from(device_size) <= (NORMAL_PX << index))
            .unwrap_or(BUCKETS.len() - 1);
        // 先找尺寸足够的档位（从小到大），再用偏小的档位（从大到小）。
        let order: Vec<&str> = BUCKETS[start..]
            .iter()
            .chain(BUCKETS[..start].iter().rev())
            .copied()
            .collect();
        order
            .iter()
            .find_map(|bucket| load_png(&self.root.join(bucket).join(&source.filename), source))
            .map_or(LoadResult::Miss, LoadResult::Thumbnail)
    }

    pub fn save_thumbnail(&self, source: &SourceThumbnail, bytes: &[u8]) -> Option<()> {
        if !source.is_current() {
            return None;
        }
        let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().ok()?;
        if reader.info().width > NORMAL_PX || reader.info().height > NORMAL_PX {
            return None;
        }
        let mut pixels = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut pixels).ok()?;
        let path = self.root.join(BUCKETS[0]).join(&source.filename);
        let mut file = atomic_tempfile(&self.root.join(BUCKETS[0]))?;
        let mut writer = png::Encoder::new(file.as_file_mut(), info.width, info.height);
        writer.set_color(info.color_type);
        writer.set_depth(info.bit_depth);
        if let Some(palette) = &reader.info().palette {
            writer.set_palette(palette.to_vec());
        }
        if let Some(transparency) = &reader.info().trns {
            writer.set_trns(transparency.to_vec());
        }
        add_shared_text_chunks(&mut writer, source)?;
        let mut writer = writer.write_header().ok()?;
        writer
            .write_image_data(&pixels[..info.buffer_size()])
            .ok()?;
        writer.finish().ok()?;
        if !source.is_current() {
            return None;
        }
        file.persist(path).ok()?;
        Some(())
    }

    /// 生成失败时写入持久标记；文件元数据变化后标记自然失效。
    pub fn save_failure(&self, source: &SourceThumbnail) -> Option<()> {
        let dir = self.root.join("fail").join(SOFTWARE);
        let path = dir.join(&source.filename);
        let mut file = atomic_tempfile(&dir)?;
        let mut writer = png::Encoder::new(file.as_file_mut(), 1, 1);
        writer.set_color(png::ColorType::Rgba);
        add_shared_text_chunks(&mut writer, source)?;
        let mut writer = writer.write_header().ok()?;
        writer.write_image_data(&[0, 0, 0, 0]).ok()?;
        writer.finish().ok()?;
        file.persist(path).ok()?;
        Some(())
    }

    /// 只统计本应用写入的条目，供偏好设置展示用量。
    pub fn disk_usage(&self) -> io::Result<u64> {
        Ok(self.our_entries()?.iter().map(|entry| entry.bytes).sum())
    }

    /// 只删除本应用写入的条目。
    pub fn clear(&self) -> io::Result<u64> {
        for entry in self.our_entries()? {
            match fs::remove_file(&entry.path) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(error),
            }
        }
        // 己有条目已全部删除。
        Ok(0)
    }

    fn our_entries(&self) -> io::Result<Vec<CacheEntry>> {
        let mut dirs: Vec<PathBuf> = BUCKETS
            .iter()
            .map(|bucket| self.root.join(bucket))
            .collect();
        // 失败标记目录只有本应用写入，可整体视为己有。
        dirs.push(self.root.join("fail").join(SOFTWARE));
        let mut entries = Vec::new();
        for dir in dirs {
            let Ok(read_dir) = fs::read_dir(&dir) else {
                continue;
            };
            for item in read_dir {
                let Ok(item) = item else {
                    continue;
                };
                let name = item.file_name();
                let name = name.as_encoded_bytes();
                if !name
                    .strip_suffix(b".png")
                    .is_some_and(|stem| stem.len() == 32 && stem.iter().all(u8::is_ascii_hexdigit))
                {
                    continue;
                }
                let Ok(metadata) = item.metadata() else {
                    continue;
                };
                if metadata.is_file() && png_software_is_ours(&item.path()) {
                    entries.push(CacheEntry {
                        path: item.path(),
                        bytes: metadata.len(),
                    });
                }
            }
        }
        Ok(entries)
    }

    fn load_failure(&self, source: &SourceThumbnail) -> bool {
        let path = self.root.join("fail").join(SOFTWARE).join(&source.filename);
        let Some(metadata) = fs::symlink_metadata(&path).ok() else {
            return false;
        };
        // 只信任自己的失败标记：其他应用的失败原因（如缺解码器）未必适用于本应用。
        metadata.is_file() && failure_marker_matches(&path, source)
    }
}

struct CacheEntry {
    path: PathBuf,
    bytes: u64,
}

/// 在目标目录内创建临时文件，持久化时原子替换，符合共享缓存的写入约定。
fn atomic_tempfile(dir: &Path) -> Option<tempfile::NamedTempFile> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .ok()?;
    tempfile::Builder::new()
        .prefix(".ktfm-thumbnail-")
        .tempfile_in(dir)
        .ok()
}

fn add_shared_text_chunks(
    writer: &mut png::Encoder<&mut fs::File>,
    source: &SourceThumbnail,
) -> Option<()> {
    let mtime = source.mtime()?;
    // 纳秒精度块是规范允许的自定义 tEXt，其他应用会忽略。
    let _ = writer.add_text_chunk("Thumb::URI".into(), source.uri.to_string());
    let _ = writer.add_text_chunk("Thumb::MTime".into(), mtime.to_string());
    let _ = writer.add_text_chunk("Thumb::Size".into(), source.size.to_string());
    let _ = writer.add_text_chunk("Software".into(), SOFTWARE.into());
    let _ = writer.add_text_chunk("ktfm::MTimeNs".into(), source.mtime_nanos().to_string());
    Some(())
}

fn load_png(path: &Path, source: &SourceThumbnail) -> Option<Vec<u8>> {
    let bytes = read_bounded(path)?;
    let mut png = png::Decoder::new(Cursor::new(&bytes)).read_info().ok()?;
    if png.info().width > MAX_CACHE_PX || png.info().height > MAX_CACHE_PX {
        return None;
    }
    // 元数据可在图像数据之后，读取到 IEND 再验证。
    png.finish().ok()?;
    let uri_matches = png_text(png.info(), "Thumb::URI") == Some(source.uri.as_str());
    let mtime_matches = match png_text(png.info(), "ktfm::MTimeNs").and_then(parse_i128) {
        // 自己写的条目按纳秒校验；其他应用的条目退回规范要求的秒级校验。
        Some(nanos) => Some(nanos) == Some(source.mtime_nanos()),
        None => png_text(png.info(), "Thumb::MTime").and_then(parse_i64) == source.mtime(),
    };
    let size_matches = png_text(png.info(), "Thumb::Size")
        .is_none_or(|value| parse_u64(value) == Some(source.size));
    (uri_matches && mtime_matches && size_matches).then_some(bytes)
}

fn failure_marker_matches(path: &Path, source: &SourceThumbnail) -> bool {
    let Some(bytes) = read_bounded(path) else {
        return false;
    };
    let Ok(mut png) = png::Decoder::new(Cursor::new(&bytes)).read_info() else {
        return false;
    };
    if png.finish().is_err() {
        return false;
    }
    let text = |key| {
        png.info()
            .uncompressed_latin1_text
            .iter()
            .find(|chunk| chunk.keyword == key)
            .map(|chunk| chunk.text.as_str())
    };
    text("Thumb::URI") == Some(source.uri.as_str())
        && text("ktfm::MTimeNs").and_then(|value| value.parse().ok()) == Some(source.mtime_nanos())
}

/// 读取 PNG 的 tEXt 字段（latin-1 解码）。
fn png_text<'a>(info: &'a png::Info<'_>, key: &str) -> Option<&'a str> {
    info.uncompressed_latin1_text
        .iter()
        .find(|chunk| chunk.keyword == key)
        .map(|chunk| chunk.text.as_str())
}

fn parse_i128(value: &str) -> Option<i128> {
    value.parse().ok()
}

fn parse_i64(value: &str) -> Option<i64> {
    value.parse().ok()
}

fn parse_u64(value: &str) -> Option<u64> {
    value.parse().ok()
}

fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    if !fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_CACHE_FILE + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_CACHE_FILE).then_some(bytes)
}

/// 只读文件开头判断归属：本应用写入的 Software 块位于图像数据之前，
/// 统计与清理无需读取整个共享缓存。
fn png_software_is_ours(path: &Path) -> bool {
    const PREFIX: usize = 16 * 1024;
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut buffer = vec![0; PREFIX];
    let Ok(read) = file.read(&mut buffer) else {
        return false;
    };
    buffer.truncate(read);
    if buffer.first_chunk::<8>() != Some(&PNG_SIGNATURE) {
        return false;
    }
    let mut pos = 8;
    while let Some(length) = chunk_length(&buffer, pos) {
        let kind = &buffer[pos + 4..pos + 8];
        // 图像数据开始后不再有归属信息。
        if kind == b"IDAT" || kind == b"IEND" {
            return false;
        }
        let end = pos + 8 + length;
        if kind == b"tEXt"
            && let Some(chunk) = buffer.get(pos + 8..end)
            && let Some(nul) = chunk.iter().position(|&byte| byte == 0)
            && chunk[..nul] == *b"Software"
            && chunk[nul + 1..] == *SOFTWARE.as_bytes()
        {
            return true;
        }
        pos = end + 4;
    }
    false
}

/// 读取位置处的 PNG 块长度；越界或长度异常返回 None。
fn chunk_length(buffer: &[u8], pos: usize) -> Option<usize> {
    let header: &[u8; 8] = buffer.get(pos..)?.first_chunk()?;
    let length = u32::from_be_bytes(*header.first_chunk::<4>()?) as usize;
    (pos + 8 + length + 4 <= buffer.len()).then_some(length)
}

impl SourceThumbnail {
    fn mtime_nanos(&self) -> i128 {
        match self.modified.duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos() as i128,
            Err(error) => -(error.duration().as_nanos() as i128),
        }
    }

    fn mtime(&self) -> Option<i64> {
        match self.modified.duration_since(UNIX_EPOCH) {
            Ok(duration) => i64::try_from(duration.as_secs()).ok(),
            Err(error) => {
                let duration = error.duration();
                let seconds = i64::try_from(duration.as_secs()).ok()?;
                Some(-seconds - i64::from(duration.subsec_nanos() != 0))
            }
        }
    }

    pub fn is_current(&self) -> bool {
        fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.is_file()
                && metadata.len() == self.size
                && metadata.modified().ok() == Some(self.modified)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn cache_at(root: &Path) -> ThumbnailCache {
        ThumbnailCache {
            root: root.to_owned(),
        }
    }

    fn encode_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![0; (width * height * 4) as usize])
            .unwrap();
        writer.finish().unwrap();
        bytes
    }

    // 复现应用真实链路：ffmpeg 实际输出的 PNG → save_thumbnail → 模拟重启后 load。
    // 同时覆盖 jpg：jpg/jpeg 按扩展名被 image2 解复用器认领，曾因白名单缺失而全部失败。
    #[test]
    fn round_trips_a_real_ffmpeg_thumbnail_across_restart() {
        let ffmpeg = std::path::Path::new("/usr/bin/ffmpeg");
        let probe = std::process::Command::new(ffmpeg)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if !probe.is_ok_and(|status| status.success()) {
            // 无 ffmpeg 的环境跳过。
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let cache = cache_at(&root.path().join("thumbnails"));
        let photo = root.path().join("photo.jpg");
        let created = std::process::Command::new(ffmpeg)
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=640x480:duration=0.04",
                "-frames:v",
                "1",
                "-y",
            ])
            .arg(&photo)
            .output()
            .unwrap();
        assert!(created.status.success(), "生成测试图片失败");
        assert!(photo.metadata().unwrap().len() > 0);

        // jpg 曾因探测为 image2 被白名单拒绝，导致照片缩略图永远生成失败。
        let bytes = crate::media::thumbnail(ffmpeg, &photo).expect("ffmpeg 应能生成 jpg 缩略图");
        let source = cache.source(&photo).expect("应能建立源");
        assert!(cache.save_thumbnail(&source, &bytes).is_some());
        // 写入 normal 桶，其他尺寸档位请求也能命中同一文件。
        assert!(
            root.path()
                .join("thumbnails/normal")
                .join(&source.filename)
                .exists()
        );

        // 模拟重启：全新实例、仅凭文件元数据加载。
        let cache = cache_at(&root.path().join("thumbnails"));
        let metadata = fs::metadata(&photo).unwrap();
        let source = cache
            .source_with_metadata(&photo, metadata.modified().unwrap(), metadata.len())
            .unwrap();
        for size in [64u16, 96, 128, 256] {
            match cache.load(&source, size) {
                LoadResult::Thumbnail(loaded) => {
                    assert!(loaded.starts_with(b"\x89PNG\r\n\x1a\n"));
                }
                _ => panic!("重启后应命中共享缓存（size={size}）"),
            }
        }
    }

    #[test]
    fn failure_marker_suppresses_retry_until_the_file_changes() {
        let root = tempfile::tempdir().unwrap();
        let cache = cache_at(&root.path().join("thumbnails"));
        let file = root.path().join("video.mp4");
        fs::write(&file, b"broken").unwrap();
        let source = cache.source(&file).unwrap();
        cache.save_failure(&source).unwrap();

        assert!(matches!(cache.load(&source, 96), LoadResult::Failed));
        assert!(
            root.path()
                .join("thumbnails/fail/ktfm")
                .join(&source.filename)
                .exists()
        );

        // 同一文件变化后：标记记录的修改时间不再匹配，重新视为缺失。
        fs::write(&file, b"longer content").unwrap();
        let handle = fs::File::options().write(true).open(&file).unwrap();
        handle
            .set_modified(SystemTime::now() + Duration::from_secs(1))
            .unwrap();
        drop(handle);
        let refreshed = cache.source(&file).unwrap();
        assert_ne!(refreshed.mtime_nanos(), source.mtime_nanos());
        assert!(matches!(cache.load(&refreshed, 96), LoadResult::Miss));
    }

    #[test]
    fn clear_removes_only_our_entries() {
        let root = tempfile::tempdir().unwrap();
        let cache = cache_at(&root.path().join("thumbnails"));
        let ours = root.path().join("ours.png");
        let theirs = root.path().join("theirs.png");
        for file in [&ours, &theirs] {
            fs::write(file, b"source").unwrap();
            let source = cache.source(file).unwrap();
            let png = encode_png(4, 4);
            if file == &ours {
                assert!(cache.save_thumbnail(&source, &png).is_some());
            } else {
                // 手写一个 Software 不同的条目，模拟其他应用写入共享缓存。
                let dir = root.path().join("thumbnails/normal");
                fs::create_dir_all(&dir).unwrap();
                let mut buffer = Vec::new();
                let mut encoder = png::Encoder::new(&mut buffer, 1, 1);
                encoder.set_color(png::ColorType::Rgba);
                let _ = encoder.add_text_chunk("Thumb::URI".into(), source.uri.to_string());
                let _ = encoder
                    .add_text_chunk("Thumb::MTime".into(), source.mtime().unwrap().to_string());
                let _ = encoder.add_text_chunk("Software".into(), "GNOME::ThumbnailFactory".into());
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&[0, 0, 0, 0]).unwrap();
                writer.finish().unwrap();
                fs::write(dir.join(&source.filename), &buffer).unwrap();
            }
        }

        assert!(cache.disk_usage().unwrap() > 0);
        assert_eq!(cache.clear().unwrap(), 0);
        let our_source = cache.source(&ours).unwrap();
        let their_source = cache.source(&theirs).unwrap();
        assert!(matches!(cache.load(&our_source, 96), LoadResult::Miss));
        assert!(matches!(
            cache.load(&their_source, 96),
            LoadResult::Thumbnail(_)
        ));
    }

    #[test]
    fn rejects_changed_sources_and_oversized_thumbnails() {
        let root = tempfile::tempdir().unwrap();
        let cache = cache_at(&root.path().join("thumbnails"));
        let file = root.path().join("含 空格.png");
        fs::write(&file, b"original").unwrap();
        let source = cache.source(&file).unwrap();

        let big = encode_png(256, 256);
        assert!(cache.save_thumbnail(&source, &big).is_none());
        assert!(matches!(cache.load(&source, 64), LoadResult::Miss));

        assert!(cache.save_thumbnail(&source, &encode_png(4, 4)).is_some());
        assert!(matches!(cache.load(&source, 64), LoadResult::Thumbnail(_)));

        fs::write(&file, b"changed and longer").unwrap();
        let stale = cache.source(&file).unwrap();
        assert!(cache.save_thumbnail(&source, &encode_png(4, 4)).is_none());
        assert!(matches!(cache.load(&stale, 64), LoadResult::Miss));
    }
}
