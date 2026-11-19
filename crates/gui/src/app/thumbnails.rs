use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{AppContext, Context, Image, ImageFormat, RenderImage, SvgRenderer};
use uuid::Uuid;

use super::App;

const MAX_CACHE_READS: usize = 16;
const MAX_GENERATIONS: usize = 4;
const CACHE_CAPACITY: usize = 256;

fn decode(bytes: Vec<u8>, renderer: &SvgRenderer) -> Option<Arc<RenderImage>> {
    Image::from_bytes(ImageFormat::Png, bytes)
        .to_image_data(renderer.clone())
        .ok()
}

// 快照更新时使用新的键，避免沿用同一路径的旧封面或失败结果。
pub(super) type ThumbnailKey = (Uuid, u64, usize);

enum ThumbnailState {
    Loading,
    NeedsGeneration,
    Ready(Option<Arc<RenderImage>>),
}

pub(super) struct Thumbnails {
    ffmpeg: Option<PathBuf>,
    #[cfg(target_os = "linux")]
    disk: Option<sys::thumbnail_cache::ThumbnailCache>,
    reading: usize,
    generating: usize,
    images: HashMap<ThumbnailKey, ThumbnailState>,
    completed: VecDeque<ThumbnailKey>,
}

impl Thumbnails {
    pub(super) fn new() -> Self {
        Self {
            ffmpeg: sys::opener::resolve_executable("ffmpeg"),
            #[cfg(target_os = "linux")]
            disk: sys::thumbnail_cache::ThumbnailCache::open(),
            reading: 0,
            generating: 0,
            images: HashMap::new(),
            completed: VecDeque::new(),
        }
    }

    pub(super) fn cache_reader(
        &self,
        renderer: SvgRenderer,
    ) -> Option<impl Fn(&Path, u16) -> Option<Arc<RenderImage>> + Send + use<>> {
        #[cfg(target_os = "linux")]
        let disk = self.disk.clone();
        #[cfg(not(target_os = "linux"))]
        let available = false;
        #[cfg(target_os = "linux")]
        let available = disk.is_some();
        available.then_some(move |path: &Path, size| {
            #[cfg(target_os = "linux")]
            {
                let disk = disk.as_ref()?;
                decode(disk.load(&disk.source(path)?, size)?, &renderer)
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = (&renderer, path, size);
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
        if !self.images.contains_key(&key) {
            self.finish(key, ThumbnailState::Ready(Some(image)), cx);
        }
    }

    fn finish(&mut self, key: ThumbnailKey, state: ThumbnailState, cx: &mut Context<App>) {
        self.completed.push_back(key);
        self.images.insert(key, state);
        if self.completed.len() > CACHE_CAPACITY
            && let Some(oldest) = self.completed.pop_front()
            && let Some(ThumbnailState::Ready(Some(image))) = self.images.remove(&oldest)
        {
            cx.drop_image(image, None);
        }
    }

    pub(super) fn get_or_request(
        &mut self,
        key: ThumbnailKey,
        path: &Path,
        device_size: u16,
        cx: &mut Context<App>,
    ) -> Option<Arc<RenderImage>> {
        let needs_generation = match self.images.get(&key) {
            Some(ThumbnailState::Ready(image)) => return image.clone(),
            Some(ThumbnailState::Loading) => return None,
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
            self.completed.retain(|candidate| *candidate != key);
        } else {
            if self.reading >= MAX_CACHE_READS {
                return None;
            }
            self.reading += 1;
        }
        let ffmpeg = self.ffmpeg.clone();
        #[cfg(target_os = "linux")]
        let disk = generate.then(|| self.disk.clone()).flatten();
        let path = path.to_owned();
        self.images.insert(key, ThumbnailState::Loading);
        let job = cx.background_spawn(async move {
            // 等待生成期间，其他窗格或程序可能已写入共享缓存。
            if let Some(image) = read_cache.and_then(|read| read(&path, device_size)) {
                return ThumbnailState::Ready(Some(image));
            }
            if !generate {
                return if ffmpeg.is_some() {
                    ThumbnailState::NeedsGeneration
                } else {
                    ThumbnailState::Ready(None)
                };
            }
            #[cfg(target_os = "linux")]
            let source = disk.as_ref().and_then(|disk| disk.source(&path));
            let image = ffmpeg.as_ref().and_then(|ffmpeg| {
                let bytes = sys::media::thumbnail(ffmpeg, &path)?;
                #[cfg(target_os = "linux")]
                if let Some((disk, source)) = disk.as_ref().zip(source.as_ref()) {
                    if !source.is_current() {
                        return None;
                    }
                    let _ = disk.save_normal(source, &bytes);
                }
                decode(bytes, &renderer)
            });
            ThumbnailState::Ready(image)
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
