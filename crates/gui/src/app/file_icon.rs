use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::*;
use gpui_kit::*;
use vfs::EntryKind;

use crate::assets::{IconOverride, file_icon_overrides};
use crate::style::icon_color;

struct ThemedFileIcon;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct IconIdentity {
    overrides: Vec<IconOverride>,
    names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct IconRequest {
    identity: IconIdentity,
    device_size: u16,
}

#[derive(Default)]
struct RecentIcons {
    images: HashMap<IconIdentity, (u16, Arc<RenderImage>)>,
    order: VecDeque<IconIdentity>,
}

impl Global for RecentIcons {}

impl RecentIcons {
    fn remember(&mut self, request: IconRequest, image: Arc<RenderImage>) {
        let IconRequest {
            identity,
            device_size,
        } = request;
        if self
            .images
            .insert(identity.clone(), (device_size, image))
            .is_none()
        {
            self.order.push_back(identity);
        }
        if self.order.len() > 256
            && let Some(oldest) = self.order.pop_front()
        {
            self.images.remove(&oldest);
        }
    }
}

impl Asset for ThemedFileIcon {
    type Source = IconRequest;
    type Output = Option<Arc<RenderImage>>;

    fn load(
        request: IconRequest,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let renderer = cx.svg_renderer();
        async move {
            let dimension = DevicePixels(i32::from(request.device_size));
            let decode = |extension: &str, bytes: &[u8]| match extension {
                "svg" => {
                    let svg = renderer.parse_svg(bytes).ok()?;
                    renderer
                        .render_parsed(&svg, SvgSize::Size(size(dimension, dimension)))
                        .ok()
                }
                "png" => Image::from_bytes(ImageFormat::Png, bytes.to_vec())
                    .to_image_data(renderer.clone())
                    .ok(),
                _ => None,
            };
            for icon in request.identity.overrides {
                if let Some((extension, bytes)) = icon.load()
                    && let Some(image) = decode(extension, &bytes)
                {
                    return Some(image);
                }
            }
            request.identity.names.iter().find_map(|name| {
                // 按实际设备像素查找，让只声明了 1× 的可缩放目录也能精确匹配。
                let path = sys::mime::themed_icon(name, request.device_size, 1)?;
                let bytes = std::fs::read(&path).ok()?;
                decode(path.extension()?.to_str()?, &bytes)
            })
        }
    }
}

pub(super) fn file_icon(
    kind: EntryKind,
    path: &Path,
    fallback: IconName,
    icon_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (overrides, names) = match kind {
        EntryKind::File => path.file_name().map_or_else(Default::default, |name| {
            let (mime, names) = sys::mime::icon_names(name);
            (file_icon_overrides(name, mime.as_deref()), names)
        }),
        EntryKind::Directory => (Vec::new(), vec!["folder".into()]),
        _ => (Vec::new(), Vec::new()),
    };
    let image = if overrides.is_empty() && names.is_empty() {
        None
    } else {
        let identity = IconIdentity { overrides, names };
        let pixels = (f32::from(icon_size) * window.scale_factor()).ceil() as u16;
        // 相近的缩放档位复用同一份图像；跨尺寸加载时沿用最近的图像。
        let device_size = pixels.max(16).checked_next_power_of_two().unwrap_or(pixels);
        let request = IconRequest {
            identity,
            device_size,
        };
        if let Some(Some(image)) = window.use_asset::<ThemedFileIcon>(&request, cx) {
            if cx
                .try_global::<RecentIcons>()
                .and_then(|cache| cache.images.get(&request.identity))
                .is_none_or(|(size, _)| *size < device_size)
            {
                cx.default_global::<RecentIcons>()
                    .remember(request, image.clone());
            }
            Some(image)
        } else {
            cx.try_global::<RecentIcons>()
                .and_then(|cache| cache.images.get(&request.identity))
                .map(|(_, image)| image.clone())
        }
    };
    if let Some(image) = image {
        img(image)
            .size(icon_size)
            .object_fit(ObjectFit::Contain)
            .flex_shrink_0()
            .into_any_element()
    } else {
        Icon::new(fallback)
            .size(icon_size)
            .flex_shrink_0()
            .text_color(icon_color(fallback, cx.theme().mode))
            .into_any_element()
    }
}
