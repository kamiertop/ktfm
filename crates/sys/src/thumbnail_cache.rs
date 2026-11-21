use std::fs;
use std::io::{self, Cursor, Read};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use rustix::fs::{FlockOperation, OFlags, flock};
use url::Url;

const BUCKETS: [&str; 4] = ["normal", "large", "x-large", "xx-large"];
const MAX_CACHE_FILE: u64 = 8 * 1024 * 1024;
const MAX_DISK_BYTES: u64 = 256 * 1024 * 1024;
const PRUNE_INTERVAL_BYTES: u64 = 4 * 1024 * 1024;
static BYTES_SINCE_PRUNE: AtomicU64 = AtomicU64::new(PRUNE_INTERVAL_BYTES);

#[derive(Clone)]
pub struct ThumbnailCache {
    private: PathBuf,
    shared: PathBuf,
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
        let base = directories::BaseDirs::new()?;
        Some(Self {
            private: base.cache_dir().join("ktfm").join("thumbnails"),
            shared: base.cache_dir().join("thumbnails"),
        })
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
        if path.starts_with(&self.private) || path.starts_with(&self.shared) {
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

    pub fn load(&self, source: &SourceThumbnail, device_size: u16) -> Option<Vec<u8>> {
        let start = BUCKETS
            .iter()
            .enumerate()
            .position(|(index, _)| device_size <= (128 << index))
            .unwrap_or(BUCKETS.len() - 1);
        let own = || load_png(&self.private.join(&source.filename), source, true);
        let shared = |indices: &[&str]| {
            indices.iter().find_map(|bucket| {
                load_png(
                    &self.shared.join(bucket).join(&source.filename),
                    source,
                    false,
                )
            })
        };
        if start == 0 {
            own().or_else(|| shared(&BUCKETS))
        } else {
            shared(&BUCKETS[start..]).or_else(own).or_else(|| {
                BUCKETS[..start].iter().rev().find_map(|bucket| {
                    load_png(
                        &self.shared.join(bucket).join(&source.filename),
                        source,
                        false,
                    )
                })
            })
        }
    }

    pub fn disk_usage(&self) -> io::Result<u64> {
        if !self.private_exists()? {
            return Ok(0);
        }
        let _lock = self.lock_private()?;
        Ok(self
            .private_entries()?
            .iter()
            .map(|entry| entry.bytes)
            .sum())
    }

    pub fn clear(&self) -> io::Result<u64> {
        if !self.private_exists()? {
            return Ok(0);
        }
        let _lock = self.lock_private()?;
        self.trim(0)
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
        let _lock = self.lock_private().ok()?;
        let mut file = tempfile::Builder::new()
            .prefix(".ktfm-thumbnail-")
            .tempfile_in(&self.private)
            .ok()?;
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
        writer
            .add_text_chunk("ktfm::MTimeNs".into(), source.mtime_nanos().to_string())
            .ok()?;
        let mut writer = writer.write_header().ok()?;
        writer
            .write_image_data(&pixels[..info.buffer_size()])
            .ok()?;
        writer.finish().ok()?;
        if !source.is_current() {
            return None;
        }
        let persisted = file.persist(self.private.join(&source.filename)).ok()?;
        let bytes = persisted.metadata().ok()?.len();
        if BYTES_SINCE_PRUNE.fetch_add(bytes, Relaxed) + bytes >= PRUNE_INTERVAL_BYTES {
            BYTES_SINCE_PRUNE.store(0, Relaxed);
            if self.trim(MAX_DISK_BYTES).is_err() {
                BYTES_SINCE_PRUNE.store(PRUNE_INTERVAL_BYTES, Relaxed);
            }
        }
        Some(())
    }

    fn private_exists(&self) -> io::Result<bool> {
        let app_dir = self.private.parent().expect("缓存目录应有父目录");
        for dir in [app_dir, self.private.as_path()] {
            match fs::symlink_metadata(dir) {
                Ok(metadata) if metadata.is_dir() => (),
                Ok(_) => return Err(io::Error::other("缩略图缓存路径需要是普通目录")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }

    fn lock_private(&self) -> io::Result<fs::File> {
        if !self.private_exists()? {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.private)?;
            self.private_exists()?;
        }
        let file = fs::OpenOptions::new()
            .mode(0o600)
            .custom_flags(OFlags::NOFOLLOW.bits() as i32)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.private.join(".lock"))?;
        flock(&file, FlockOperation::LockExclusive)?;
        Ok(file)
    }

    fn private_entries(&self) -> io::Result<Vec<CacheEntry>> {
        if !self.private_exists()? {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.private)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.as_encoded_bytes();
            let our_png = name
                .strip_suffix(b".png")
                .is_some_and(|stem| stem.len() == 32 && stem.iter().all(u8::is_ascii_hexdigit));
            let temporary = name.starts_with(b".ktfm-thumbnail-");
            if !our_png && !temporary {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if metadata.is_file() {
                entries.push(CacheEntry {
                    path: entry.path(),
                    bytes: metadata.len(),
                    modified: metadata.modified().unwrap_or(UNIX_EPOCH),
                });
            }
        }
        Ok(entries)
    }

    // 写入、淘汰和清理共用一把进程间锁，避免删掉正在写入的临时文件。
    fn trim(&self, limit: u64) -> io::Result<u64> {
        let mut entries = self.private_entries()?;
        let mut remaining: u64 = entries.iter().map(|entry| entry.bytes).sum();
        if limit != 0 && remaining <= limit {
            return Ok(remaining);
        }
        entries.sort_unstable_by_key(|entry| entry.modified);
        for entry in entries {
            if limit != 0 && remaining <= limit {
                break;
            }
            match fs::remove_file(entry.path) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(error),
            }
            remaining -= entry.bytes;
        }
        Ok(remaining)
    }
}

struct CacheEntry {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

fn load_png(path: &Path, source: &SourceThumbnail, private: bool) -> Option<Vec<u8>> {
    if !fs::symlink_metadata(path).ok()?.is_file() {
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
    let version_matches = if private {
        text("ktfm::MTimeNs").and_then(|value| value.parse().ok()) == Some(source.mtime_nanos())
    } else {
        text("Thumb::MTime").and_then(|value| value.parse().ok()) == Some(source.mtime()?)
    };
    (version_matches
        && text("Thumb::URI") == Some(source.uri.as_str())
        && text("Thumb::Size").is_none_or(|value| value.parse().ok() == Some(source.size)))
    .then_some(bytes)
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
    use std::os::unix::fs::symlink;
    use std::time::Duration;

    #[test]
    fn trims_oldest_private_entries_and_clears_without_touching_shared_cache() {
        let root = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache {
            private: root.path().join("ktfm/thumbnails"),
            shared: root.path().join("thumbnails"),
        };
        assert_eq!(cache.disk_usage().unwrap(), 0);
        fs::create_dir_all(&cache.private).unwrap();
        fs::create_dir_all(&cache.shared).unwrap();
        let old = cache.private.join("a".repeat(32) + ".png");
        let new = cache.private.join("b".repeat(32) + ".png");
        let shared = cache.shared.join("c".repeat(32) + ".png");
        let unrelated = cache.private.join("keep.txt");
        fs::write(&old, [0; 4]).unwrap();
        fs::File::open(&old)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(1))
            .unwrap();
        fs::write(&new, [0; 4]).unwrap();
        fs::write(&shared, [1; 4]).unwrap();
        fs::write(&unrelated, [2; 4]).unwrap();
        symlink(&shared, cache.private.join("d".repeat(32) + ".png")).unwrap();

        assert_eq!(cache.disk_usage().unwrap(), 8);
        assert_eq!(cache.trim(4).unwrap(), 4);
        assert!(!old.exists());
        assert!(new.exists());
        assert_eq!(cache.clear().unwrap(), 0);
        assert_eq!(cache.disk_usage().unwrap(), 0);
        assert_eq!(fs::read(&shared).unwrap(), [1; 4]);
        assert_eq!(fs::read(&unrelated).unwrap(), [2; 4]);
    }

    #[test]
    fn refuses_to_clear_a_private_directory_linked_to_shared_cache() {
        let root = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache {
            private: root.path().join("ktfm/thumbnails"),
            shared: root.path().join("thumbnails"),
        };
        fs::create_dir(cache.private.parent().unwrap()).unwrap();
        fs::create_dir(&cache.shared).unwrap();
        let shared_file = cache.shared.join("a".repeat(32) + ".png");
        fs::write(&shared_file, [1]).unwrap();
        symlink(&cache.shared, &cache.private).unwrap();
        assert!(cache.clear().is_err());
        assert_eq!(fs::read(shared_file).unwrap(), [1]);
    }

    #[test]
    fn loads_current_thumbnails_and_rejects_changed_sources() {
        let root = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache {
            private: root.path().join("ktfm/thumbnails"),
            shared: root.path().join("thumbnails"),
        };
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
        let cache = ThumbnailCache {
            private: root.path().join("ktfm/thumbnails"),
            shared: root.path().join("thumbnails"),
        };
        let metadata = fs::metadata(&file).unwrap();
        let snapshot_source = cache
            .source_with_metadata(&file, metadata.modified().unwrap(), metadata.len())
            .unwrap();
        assert!(cache.load(&snapshot_source, 64).is_some());
        assert!(cache.load(&source, 64).is_some());
        assert!(cache.load(&source, 256).is_some());
        let normal = cache.private.join(&source.filename);
        let large = cache.shared.join("large");
        fs::create_dir_all(&large).unwrap();
        fs::copy(&normal, large.join(&source.filename)).unwrap();
        fs::write(normal, b"corrupt thumbnail").unwrap();
        assert!(cache.load(&source, 64).is_some());

        fs::write(&file, b"changed and longer").unwrap();
        assert!(cache.save_normal(&source, &bytes).is_none());
        let changed = cache.source(&file).unwrap();
        assert!(cache.load(&changed, 64).is_none());
    }
}
