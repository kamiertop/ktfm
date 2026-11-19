use std::fs;
use std::io::{Cursor, Read};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use url::Url;

const BUCKETS: [&str; 4] = ["normal", "large", "x-large", "xx-large"];
const MAX_CACHE_FILE: u64 = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct ThumbnailCache(PathBuf);

pub struct SourceThumbnail {
    path: PathBuf,
    uri: Url,
    filename: String,
    modified: SystemTime,
    size: u64,
}

impl ThumbnailCache {
    pub fn open() -> Option<Self> {
        Some(Self(
            directories::BaseDirs::new()?.cache_dir().join("thumbnails"),
        ))
    }

    pub fn source(&self, path: &Path) -> Option<SourceThumbnail> {
        if path.starts_with(&self.0) {
            return None;
        }
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let uri = Url::from_file_path(path).ok()?;
        let filename = format!("{:x}.png", Md5::digest(uri.as_str().as_bytes()));
        Some(SourceThumbnail {
            path: path.to_owned(),
            uri,
            filename,
            modified: metadata.modified().ok()?,
            size: metadata.len(),
        })
    }

    pub fn load(&self, source: &SourceThumbnail, device_size: u16) -> Option<Vec<u8>> {
        let start = BUCKETS
            .iter()
            .enumerate()
            .position(|(index, _)| device_size <= (128 << index))
            .unwrap_or(BUCKETS.len() - 1);
        let mtime = source.mtime()?;
        BUCKETS[start..]
            .iter()
            .chain(BUCKETS[..start].iter().rev())
            .find_map(|bucket| {
                let path = self.0.join(bucket).join(&source.filename);
                if !fs::symlink_metadata(&path).ok()?.is_file() {
                    return None;
                }
                let mut bytes = Vec::new();
                fs::File::open(path)
                    .ok()?
                    .take(MAX_CACHE_FILE + 1)
                    .read_to_end(&mut bytes)
                    .ok()?;
                if bytes.len() as u64 > MAX_CACHE_FILE {
                    return None;
                }
                let mut png = png::Decoder::new(Cursor::new(&bytes)).read_info().ok()?;
                if png.info().width > 1024 || png.info().height > 1024 {
                    return None;
                }
                // 元数据可在图像数据之后，读取到 IEND 再验证。
                png.finish().ok()?;
                let text = |key| {
                    png.info()
                        .uncompressed_latin1_text
                        .iter()
                        .find(|chunk| chunk.keyword == key)
                        .map(|chunk| chunk.text.as_str())
                };
                (text("Thumb::URI") == Some(source.uri.as_str())
                    && text("Thumb::MTime").and_then(|value| value.parse().ok()) == Some(mtime)
                    && text("Thumb::Size")
                        .is_none_or(|value| value.parse().ok() == Some(source.size)))
                .then_some(bytes)
            })
    }

    pub fn save_normal(&self, source: &SourceThumbnail, bytes: &[u8]) -> Option<()> {
        if !source.is_current() {
            return None;
        }
        let mtime = source.mtime()?;
        let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().ok()?;
        if reader.info().width > 128 || reader.info().height > 128 {
            return None;
        }
        let mut pixels = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut pixels).ok()?;
        let dir = self.0.join("normal");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .ok()?;
        let mut file = tempfile::NamedTempFile::new_in(&dir).ok()?;
        let mut writer = png::Encoder::new(file.as_file_mut(), info.width, info.height);
        writer.set_color(info.color_type);
        writer.set_depth(info.bit_depth);
        if let Some(palette) = &reader.info().palette {
            writer.set_palette(palette.to_vec());
        }
        if let Some(transparency) = &reader.info().trns {
            writer.set_trns(transparency.to_vec());
        }
        writer
            .add_text_chunk("Thumb::URI".into(), source.uri.to_string())
            .ok()?;
        writer
            .add_text_chunk("Thumb::MTime".into(), mtime.to_string())
            .ok()?;
        writer
            .add_text_chunk("Thumb::Size".into(), source.size.to_string())
            .ok()?;
        let mut writer = writer.write_header().ok()?;
        writer
            .write_image_data(&pixels[..info.buffer_size()])
            .ok()?;
        writer.finish().ok()?;
        if !source.is_current() {
            return None;
        }
        file.persist(dir.join(&source.filename)).ok()?;
        Some(())
    }
}

impl SourceThumbnail {
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

    #[test]
    fn loads_current_thumbnails_and_rejects_changed_sources() {
        let root = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache(root.path().join("thumbnails"));
        let file = root.path().join("含 空格.png");
        fs::write(&file, b"original").unwrap();
        let source = cache.source(&file).unwrap();
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 128, 255, 255]).unwrap();
            writer.finish().unwrap();
        }
        assert!(cache.load(&source, 64).is_none());
        cache.save_normal(&source, &bytes).unwrap();
        assert!(cache.load(&source, 64).is_some());
        assert!(cache.load(&source, 256).is_some());
        let normal = cache.0.join("normal").join(&source.filename);
        let large = cache.0.join("large");
        fs::create_dir(&large).unwrap();
        fs::copy(&normal, large.join(&source.filename)).unwrap();
        fs::write(normal, b"corrupt thumbnail").unwrap();
        assert!(cache.load(&source, 64).is_some());

        fs::write(&file, b"changed and longer").unwrap();
        assert!(cache.save_normal(&source, &bytes).is_none());
        let changed = cache.source(&file).unwrap();
        assert!(cache.load(&changed, 64).is_none());
    }
}
