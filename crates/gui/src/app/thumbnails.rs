use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::{AppContext, Context, Image, ImageFormat, RenderImage, SvgRenderer};
use uuid::Uuid;
use vfs::Entry;

use super::App;

const MAX_CACHE_READS: usize = 16;
const MAX_GENERATIONS: usize = 4;
const CACHE_BYTE_LIMIT: usize = 64 * 1024 * 1024;
const CACHE_RECORD_LIMIT: usize = 2048;

fn decode(bytes: Vec<u8>, renderer: &SvgRenderer) -> Option<Arc<RenderImage>> {
    Image::from_bytes(ImageFormat::Png, bytes)
        .to_image_data(renderer.clone())
        .ok()
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ThumbnailKey {
    path: PathBuf,
    version: FileVersion,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum FileVersion {
    Metadata {
        modified: SystemTime,
        size: u64,
        created: Option<SystemTime>,
    },
    Snapshot {
        pane: Uuid,
        revision: u64,
    },
}

impl ThumbnailKey {
    pub(super) fn new(entry: &Entry, pane: Uuid, revision: u64) -> Self {
        let version = match (entry.modified, entry.size) {
            (Some(modified), Some(size)) => FileVersion::Metadata {
                modified,
                size,
                created: entry.created,
            },
            _ => FileVersion::Snapshot { pane, revision },
        };
        Self {
            path: entry.path.clone(),
            version,
        }
    }

    fn is_current(&self) -> bool {
        let FileVersion::Metadata {
            modified,
            size,
            created,
        } = self.version
        else {
            return true;
        };
        std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.is_file()
                && metadata.len() == size
                && metadata.modified().ok() == Some(modified)
                && metadata.created().ok() == created
        })
    }
}

enum ThumbnailState {
    NeedsGeneration,
    Ready(Arc<RenderImage>),
    Unavailable(Instant),
}

#[derive(Default)]
struct ThumbnailMemory {
    entries: HashMap<ThumbnailKey, CachedThumbnail>,
    bytes: usize,
    clock: u64,
}

struct CachedThumbnail {
    state: ThumbnailState,
    bytes: usize,
    last_used: u64,
}

impl ThumbnailMemory {
    fn get(&mut self, key: &ThumbnailKey) -> Option<&ThumbnailState> {
        let entry = self.entries.get_mut(key)?;
        self.clock += 1;
        entry.last_used = self.clock;
        Some(&entry.state)
    }

    fn remove(&mut self, key: &ThumbnailKey) -> Option<ThumbnailState> {
        let entry = self.entries.remove(key)?;
        self.bytes -= entry.bytes;
        Some(entry.state)
    }

    fn insert(&mut self, key: ThumbnailKey, state: ThumbnailState) {
        let bytes = match &state {
            ThumbnailState::Ready(image) => (0..image.frame_count())
                .filter_map(|frame| image.as_bytes(frame).map(<[u8]>::len))
                .sum(),
            _ => 0,
        };
        self.clock += 1;
        self.remove(&key);
        self.entries.insert(
            key,
            CachedThumbnail {
                state,
                bytes,
                last_used: self.clock,
            },
        );
        self.bytes += bytes;
    }

    fn evict(&mut self, byte_limit: usize, record_limit: usize) -> Vec<Arc<RenderImage>> {
        let mut images = Vec::new();
        while self.bytes > byte_limit || self.entries.len() > record_limit {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
                .expect("超过上限时至少有一个缓存项");
            if let Some(ThumbnailState::Ready(image)) = self.remove(&oldest) {
                images.push(image);
            }
        }
        images
    }
}

pub(super) struct Thumbnails {
    ffmpeg: Option<PathBuf>,
    #[cfg(target_os = "linux")]
    disk: Option<sys::thumbnail_cache::ThumbnailCache>,
    reading: usize,
    generating: usize,
    memory: ThumbnailMemory,
    pending: HashSet<ThumbnailKey>,
}

impl Thumbnails {
    pub(super) fn new() -> Self {
        Self {
            ffmpeg: sys::opener::resolve_executable("ffmpeg"),
            #[cfg(target_os = "linux")]
            disk: sys::thumbnail_cache::ThumbnailCache::open(),
            reading: 0,
            generating: 0,
            memory: ThumbnailMemory::default(),
            pending: HashSet::new(),
        }
    }

    pub(super) fn cache_reader(
        &self,
        renderer: SvgRenderer,
    ) -> Option<impl Fn(&ThumbnailKey, u16) -> Option<Arc<RenderImage>> + Send + Sync + use<>> {
        #[cfg(target_os = "linux")]
        let disk = self.disk.clone();
        #[cfg(not(target_os = "linux"))]
        let available = false;
        #[cfg(target_os = "linux")]
        let available = disk.is_some();
        available.then_some(move |key: &ThumbnailKey, size| {
            #[cfg(target_os = "linux")]
            {
                let disk = disk.as_ref()?;
                let source = match key.version {
                    FileVersion::Metadata { modified, size, .. } => {
                        disk.source_with_metadata(&key.path, modified, size)?
                    }
                    FileVersion::Snapshot { .. } => disk.source(&key.path)?,
                };
                decode(disk.load(&source, size)?, &renderer)
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = (&renderer, key, size);
                None
            }
        })
    }

    pub(super) fn seed_cached(
        &mut self,
        key: ThumbnailKey,
        image: Arc<RenderImage>,
        cx: &mut Context<App>,
    ) {
        if !self.pending.contains(&key) && !self.memory.entries.contains_key(&key) {
            self.finish(key, ThumbnailState::Ready(image), cx);
        }
    }

    fn finish(&mut self, key: ThumbnailKey, state: ThumbnailState, cx: &mut Context<App>) {
        self.pending.remove(&key);
        self.memory.insert(key, state);
        for image in self.memory.evict(CACHE_BYTE_LIMIT, CACHE_RECORD_LIMIT) {
            cx.drop_image(image, None);
        }
    }

    pub(super) fn get_or_request(
        &mut self,
        key: ThumbnailKey,
        device_size: u16,
        cx: &mut Context<App>,
    ) -> Option<Arc<RenderImage>> {
        if self.pending.contains(&key) {
            return None;
        }
        let needs_generation = match self.memory.get(&key) {
            Some(ThumbnailState::Ready(image)) => return Some(image.clone()),
            Some(ThumbnailState::Unavailable(at)) if at.elapsed() < Duration::from_secs(5) => {
                return None;
            }
            Some(ThumbnailState::Unavailable(_)) => false,
            Some(ThumbnailState::NeedsGeneration) => true,
            None => false,
        };
        let renderer = cx.svg_renderer();
        let read_cache = self.cache_reader(renderer.clone());
        let generate = needs_generation || read_cache.is_none();
        if generate {
            self.ffmpeg.as_ref()?;
            if self.generating >= MAX_GENERATIONS {
                return None;
            }
            self.generating += 1;
        } else {
            if self.reading >= MAX_CACHE_READS {
                return None;
            }
            self.reading += 1;
        }
        let ffmpeg = self.ffmpeg.clone();
        #[cfg(target_os = "linux")]
        let disk = generate.then(|| self.disk.clone()).flatten();
        let path = key.path.clone();
        self.memory.remove(&key);
        self.pending.insert(key.clone());
        let source_key = key.clone();
        let job = cx.background_spawn(async move {
            // 等待生成期间，其他窗口或程序可能已生成缩略图。
            if let Some(image) = read_cache.and_then(|read| read(&source_key, device_size)) {
                return ThumbnailState::Ready(image);
            }
            if !generate {
                return if ffmpeg.is_some() {
                    ThumbnailState::NeedsGeneration
                } else {
                    ThumbnailState::Unavailable(Instant::now())
                };
            }
            #[cfg(target_os = "linux")]
            let source = disk.as_ref().and_then(|disk| disk.source(&path));
            let image = ffmpeg.as_ref().and_then(|ffmpeg| {
                if !source_key.is_current() {
                    return None;
                }
                let bytes = sys::media::thumbnail(ffmpeg, &path)?;
                if !source_key.is_current() {
                    return None;
                }
                #[cfg(target_os = "linux")]
                if let Some((disk, source)) = disk.as_ref().zip(source.as_ref()) {
                    if !source.is_current() {
                        return None;
                    }
                    let _ = disk.save_normal(source, &bytes);
                }
                decode(bytes, &renderer)
            });
            image.map_or_else(
                || ThumbnailState::Unavailable(Instant::now()),
                ThumbnailState::Ready,
            )
        });
        cx.spawn(async move |app, cx| {
            let state = job.await;
            let _ = app.update(cx, |app, cx| {
                let cache = &mut app.thumbnails;
                if generate {
                    cache.generating -= 1;
                } else {
                    cache.reading -= 1;
                }
                // 未生成的候选也要参与淘汰，避免快速切换目录时累积。
                cache.finish(key, state, cx);
                cx.notify();
            });
        })
        .detach();
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    use gpui_kit::SvgRenderer;
    use uuid::Uuid;
    use vfs::{Entry, EntryKind};

    use super::{FileVersion, ThumbnailKey, ThumbnailMemory, ThumbnailState, decode};

    #[test]
    fn evicts_the_least_recently_used_image_at_the_decoded_byte_limit() {
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\
            \x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\x0d\
            IDAT\x78\x9c\x63\x60\x68\xf8\xff\x1f\x00\x04\x82\x02\x7f\xe7\xd8\x5a\x6c\
            \x00\x00\x00\x00IEND\xae\x42\x60\x82";
        let renderer = SvgRenderer::new(Arc::new(()));
        let images: Vec<_> = (0..3)
            .map(|_| decode(png.to_vec(), &renderer).unwrap())
            .collect();
        let pane = Uuid::new_v4();
        let keys: Vec<_> = (0..3)
            .map(|i| ThumbnailKey {
                path: format!("/images/{i}.png").into(),
                version: FileVersion::Snapshot { pane, revision: 1 },
            })
            .collect();
        let mut memory = ThumbnailMemory::default();
        memory.insert(keys[0].clone(), ThumbnailState::Ready(images[0].clone()));
        memory.insert(keys[1].clone(), ThumbnailState::Ready(images[1].clone()));
        assert!(memory.get(&keys[0]).is_some());
        memory.insert(keys[2].clone(), ThumbnailState::Ready(images[2].clone()));

        assert_eq!(memory.bytes, 12);
        assert_eq!(memory.evict(8, 10), [images[1].clone()]);
        assert_eq!(memory.bytes, 8);
        assert!(memory.get(&keys[0]).is_some());
        assert!(memory.get(&keys[1]).is_none());
        assert!(memory.get(&keys[2]).is_some());
    }

    #[test]
    fn reuses_unchanged_images_across_snapshots_and_panes() {
        let mut entry = Entry {
            name: "image.png".into(),
            path: "/photos/image.png".into(),
            kind: EntryKind::File,
            size: Some(4096),
            modified: Some(UNIX_EPOCH),
            created: None,
            accessed: None,
            access: None,
        };
        let pane = Uuid::new_v4();
        let before = ThumbnailKey::new(&entry, pane, 1);
        assert_eq!(before, ThumbnailKey::new(&entry, Uuid::new_v4(), 8));
        entry.modified = Some(UNIX_EPOCH + Duration::from_nanos(1));
        assert_ne!(before, ThumbnailKey::new(&entry, pane, 2));
        entry.modified = None;
        assert_ne!(
            ThumbnailKey::new(&entry, pane, 2),
            ThumbnailKey::new(&entry, pane, 3)
        );
    }
}
