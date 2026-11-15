use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::{App, Hsla, Pixels, px, rgb};

pub const APP_ID: &str = "io.github.kamiertop.ktfm";

pub const WINDOW_WIDTH: Pixels = px(1100.);
pub const WINDOW_HEIGHT: Pixels = px(700.);
pub const MIN_WINDOW_WIDTH: Pixels = px(640.);
pub const MIN_WINDOW_HEIGHT: Pixels = px(400.);
pub const TITLE_BAR_HEIGHT: Pixels = px(48.);
pub const SEARCH_MAX_WIDTH: Pixels = px(320.);
pub const TITLE_ICON_SIZE: Pixels = px(18.);
pub const SIDEBAR_WIDTH: Pixels = px(160.);
pub const SIDEBAR_ICON_SIZE: Pixels = px(18.);
const SIDEBAR_DIVIDER_LIGHT: u32 = 0x43816d;
const SIDEBAR_DIVIDER_DARK: u32 = 0x66b59b;
pub const FILE_ICON_SIZE: Pixels = px(20.);
pub const GRID_ICON_SIZE: Pixels = px(34.);
pub const GRID_TILE_WIDTH: Pixels = px(138.);
pub const GRID_TILE_HEIGHT: Pixels = px(102.);

pub const WINDOW_RADIUS: Pixels = px(18.);
pub const WINDOW_RESIZE_EDGE: Pixels = px(5.);
pub const WINDOW_RESIZE_CORNER: Pixels = px(12.);
pub const WINDOW_CONTROL_ICON_SIZE: Pixels = px(16.);

struct Palette {
    sidebar: &'static str,
    selection: &'static str,
    hover: &'static str,
}

const LIGHT: Palette = Palette {
    sidebar: "#edf3fa",
    selection: "#d4e6fc",
    hover: "#dce7f4",
};

const DARK: Palette = Palette {
    sidebar: "#1b2534",
    selection: "#2b4567",
    hover: "#30435d",
};

pub fn install_themes(cx: &mut App) {
    let mode = cx.theme().mode;
    Theme::update(cx, |theme| {
        for (config, palette) in [
            (&mut theme.light_theme, LIGHT),
            (&mut theme.dark_theme, DARK),
        ] {
            let config = Rc::make_mut(config);
            config.colors.sidebar = Some(palette.sidebar.into());
            config.colors.sidebar_accent = Some(palette.selection.into());
            config.colors.sidebar_accent_foreground = config.colors.sidebar_foreground.clone();
            config.colors.list_hover = Some(palette.hover.into());
        }
    });
    Theme::change(mode, None, cx);
}

pub fn sidebar_divider_color(mode: ThemeMode) -> Hsla {
    rgb(if mode.is_dark() {
        SIDEBAR_DIVIDER_DARK
    } else {
        SIDEBAR_DIVIDER_LIGHT
    })
    .into()
}

pub fn icon_color(icon: IconName, mode: ThemeMode) -> Hsla {
    let (light, dark) = match icon {
        IconName::Folder | IconName::Star | IconName::FileArchive => (0x97600d, 0xf2bd68),
        IconName::Images | IconName::Image | IconName::Download => (0x127463, 0x69d6be),
        IconName::Music | IconName::FileMusic | IconName::Tag => (0x7850aa, 0xc7a0f2),
        IconName::Film | IconName::Trash => (0xab455c, 0xf29bad),
        _ => (0x3667a3, 0x8cbcf5),
    };
    rgb(if mode.is_dark() { dark } else { light }).into()
}

pub fn sync_menu_theme(cx: &mut App) {
    use ely_core::theme::{Mode, Theme as ElyTheme};

    let mode = if cx.theme().mode.is_dark() {
        Mode::Dark
    } else {
        Mode::Light
    };
    if cx.global::<ElyTheme>().mode() != mode {
        ElyTheme::set_mode_now(mode, cx);
    }
}
