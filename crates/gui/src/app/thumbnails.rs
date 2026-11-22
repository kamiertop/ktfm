//! 缩略图的内存缓存与异步加载管道。
//!
//! 流程一句话：内存命中直接显示；否则排队一个任务，任务内先查共享磁盘缓存，
//! 未命中且未失败标记的再交给 ffmpeg 生成并落盘，结果回填内存后触发重绘。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::{AppContext, Context, Image, ImageFormat, RenderImage, SvgRenderer};
use vfs::Entry;

use super::App;

/// 磁盘缓存读取很便宜，放宽并发让重启后可见项一波读完。
const MAX_READS: usize = 32;
/// ffmpeg 进程开销大，生成并发保持克制。
const MAX_GENERATIONS: usize = 8;
const CACHE_BYTE_LIMIT: usize = 64 * 1024 * 1024;
const CACHE_RECORD_LIMIT: usize = 2048;

fn decode(bytes: Vec<u8>, renderer: &SvgRenderer) -> Option<Arc<RenderImage>> {
    Image::from_bytes(ImageFormat::Png, bytes)
        .to_image_data(renderer.clone())
        .ok()
}

/// 缩略图以文件元数据为键，跨面板、跨快照、跨会话复用；
/// 缺少元数据的条目（读取时 lstat 已失败）不生成缩略图。
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ThumbnailKey {
    path: PathBuf,
    modified: SystemTime,
    size: u64,
    created: Option<SystemTime>,
}

impl ThumbnailKey {
    pub(super) fn new(entry: &Entry) -> Option<Self> {
        Some(Self {
            path: entry.path.clone(),
            modified: entry.modified?,
            size: entry.size?,
            created: entry.created,
        })
    }

    fn is_current(&self) -> bool {
        std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.is_file()
                && metadata.len() == self.size
                && metadata.modified().ok() == Some(self.modified)
                && metadata.created().ok() == self.created
        })
    }
}

enum ThumbnailState {
    Ready(Arc<RenderImage>),
    /// 本会话内不再尝试；失败标记已写入磁盘时，重启后同样直接失败，
    /// 文件元数据变化后键改变，自然重新尝试。
    Failed,
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
            ThumbnailState::Failed => 0,
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
    disk: Option<sys::thumbnail_cache::ThumbnailCache>,
    reads: usize,
    generating: usize,
    backlog: VecDeque<ThumbnailKey>,
    memory: ThumbnailMemory,
    pending: HashSet<ThumbnailKey>,
}

impl Thumbnails {
    pub(super) fn new() -> Self {
        Self {
            ffmpeg: sys::opener::resolve_executable("ffmpeg"),
            disk: sys::thumbnail_cache::ThumbnailCache::open(),
            reads: 0,
            generating: 0,
            backlog: VecDeque::new(),
            memory: ThumbnailMemory::default(),
            pending: HashSet::new(),
        }
    }

    /// 渲染期间对可见条目调用：命中返回图片；未命中时启动后台任务并在
    /// 任务完成后通过 notify 重绘。并发已满时返回 None，等待下一个
    /// 任务完成触发的重绘再排队。
    pub(super) fn get_or_request(
        &mut self,
        key: ThumbnailKey,
        device_size: u16,
        cx: &mut Context<App>,
    ) -> Option<Arc<RenderImage>> {
        match self.memory.get(&key) {
            Some(ThumbnailState::Ready(image)) => return Some(image.clone()),
            Some(ThumbnailState::Failed) => return None,
            None => (),
        }
        if self.pending.contains(&key) || self.reads >= MAX_READS {
            return None;
        }
        self.pending.insert(key.clone());
        self.reads += 1;
        let disk = self.disk.clone();
        let path = key.path.clone();
        let renderer = cx.svg_renderer();
        let job_key = key.clone();
        let job = cx.background_spawn(async move {
            // 等待期间其他窗口或程序可能已生成缩略图，先查共享缓存。
            if let Some(disk) = disk.as_ref()
                && let Some(source) =
                    disk.source_with_metadata(&path, job_key.modified, job_key.size)
            {
                match disk.load(&source, device_size) {
                    sys::thumbnail_cache::LoadResult::Thumbnail(bytes) => {
                        if let Some(image) = decode(bytes, &renderer) {
                            return Some(ThumbnailState::Ready(image));
                        }
                    }
                    // 命中失败标记：文件未变期间不再尝试。
                    sys::thumbnail_cache::LoadResult::Failed => {
                        return Some(ThumbnailState::Failed);
                    }
                    sys::thumbnail_cache::LoadResult::Miss => (),
                }
            }
            // 缓存未命中，交给生成队列。
            None
        });
        cx.spawn(async move |app, cx| {
            let state = job.await;
            let _ = app.update(cx, |app, cx| {
                let cache = &mut app.thumbnails;
                cache.reads -= 1;
                match state {
                    Some(state) => cache.finish(key, state, cx),
                    // 未安装 ffmpeg 是环境问题而非文件问题，不进生成队列，
                    // 也不写失败标记，下次启动环境变化后可恢复。
                    None if cache.ffmpeg.is_none() => cache.finish(key, ThumbnailState::Failed, cx),
                    None => cache.backlog.push_back(key),
                }
                cache.pump(cx);
                cx.notify();
            });
        })
        .detach();
        None
    }

    /// 从积压队列按剩余并发槽位拉起 ffmpeg 生成任务。
    fn pump(&mut self, cx: &mut Context<App>) {
        while self.generating < MAX_GENERATIONS
            && let Some(key) = self.backlog.pop_front()
        {
            self.generating += 1;
            let disk = self.disk.clone();
            let ffmpeg = self.ffmpeg.clone();
            let path = key.path.clone();
            let renderer = cx.svg_renderer();
            let job_key = key.clone();
            let job = cx.background_spawn(async move {
                let Some(ffmpeg) = ffmpeg else {
                    return ThumbnailState::Failed;
                };
                match sys::media::thumbnail(&ffmpeg, &path) {
                    Some(bytes) => {
                        // 生成期间文件可能被替换，校验未变才落盘。
                        if job_key.is_current()
                            && let Some(disk) = disk.as_ref()
                            && let Some(source) = disk.source(&path)
                        {
                            let _ = disk.save_thumbnail(&source, &bytes);
                        }
                        decode(bytes, &renderer)
                            .map_or(ThumbnailState::Failed, ThumbnailState::Ready)
                    }
                    None => {
                        if job_key.is_current()
                            && let Some(disk) = disk.as_ref()
                            && let Some(source) = disk.source(&path)
                        {
                            let _ = disk.save_failure(&source);
                        }
                        ThumbnailState::Failed
                    }
                }
            });
            cx.spawn(async move |app, cx| {
                let state = job.await;
                let _ = app.update(cx, |app, cx| {
                    let cache = &mut app.thumbnails;
                    cache.generating -= 1;
                    cache.finish(key, state, cx);
                    cache.pump(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn finish(&mut self, key: ThumbnailKey, state: ThumbnailState, cx: &mut Context<App>) {
        self.memory.insert(key, state);
        for image in self.memory.evict(CACHE_BYTE_LIMIT, CACHE_RECORD_LIMIT) {
            cx.drop_image(image, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    use gpui_kit::SvgRenderer;
    use vfs::{Entry, EntryKind};

    use super::{ThumbnailKey, ThumbnailMemory, ThumbnailState, decode};

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
        let keys: Vec<_> = (0..3)
            .map(|i| ThumbnailKey {
                path: format!("/images/{i}.png").into(),
                modified: UNIX_EPOCH,
                size: 4096,
                created: None,
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
    fn keys_follow_file_metadata_and_skip_entries_without_it() {
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
        let before = ThumbnailKey::new(&entry).unwrap();
        assert_eq!(before, ThumbnailKey::new(&entry).unwrap());
        entry.modified = Some(UNIX_EPOCH + Duration::from_nanos(1));
        assert_ne!(before, ThumbnailKey::new(&entry).unwrap());
        entry.modified = None;
        assert!(ThumbnailKey::new(&entry).is_none());
    }
}
