use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::{App, Hsla, Pixels, px, rgb};

pub const APP_ID: &str = "io.github.kamiertop.ktfm";

pub const WINDOW_WIDTH: Pixels = px(1200.);
pub const WINDOW_HEIGHT: Pixels = px(900.);
pub const MIN_WINDOW_WIDTH: Pixels = px(640.);
pub const MIN_WINDOW_HEIGHT: Pixels = px(400.);
pub const TITLE_BAR_HEIGHT: Pixels = px(48.);
pub const SEARCH_MAX_WIDTH: Pixels = px(320.);
pub const TITLE_ICON_SIZE: Pixels = px(18.);
pub const SIDEBAR_WIDTH: Pixels = px(160.);
pub const SIDEBAR_ICON_SIZE: Pixels = px(18.);
const SIDEBAR_DIVIDER_LIGHT: u32 = 0x43816d;
const SIDEBAR_DIVIDER_DARK: u32 = 0x66b59b;

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

pub fn file_selection_color(mode: ThemeMode) -> Hsla {
    rgb(if mode.is_dark() { 0x36577d } else { 0xb4d3f7 }).into()
}

pub struct PreferencesPalette {
    pub background: Hsla,
    pub sidebar: Hsla,
    pub border: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub selected_text: Hsla,
}

pub fn preferences_palette(mode: ThemeMode) -> PreferencesPalette {
    let (background, sidebar, border, hover, selected, selected_text) = if mode.is_dark() {
        (0x171e20, 0x202a29, 0x344341, 0x2c3b38, 0x39564d, 0xf1faf7)
    } else {
        (0xfcfdfd, 0xeff7f4, 0xdbe8e3, 0xe4f0eb, 0xd0e7de, 0x203c34)
    };
    PreferencesPalette {
        background: rgb(background).into(),
        sidebar: rgb(sidebar).into(),
        border: rgb(border).into(),
        hover: rgb(hover).into(),
        selected: rgb(selected).into(),
        selected_text: rgb(selected_text).into(),
    }
}

pub fn icon_color(icon: IconName, mode: ThemeMode) -> Hsla {
    let (light, dark) = match icon {
        IconName::Folder | IconName::Star | IconName::FileArchive => (0x97600d, 0xf2bd68),
        IconName::Images | IconName::Image | IconName::Download => (0x127463, 0x69d6be),
        IconName::Music | IconName::FileMusic | IconName::Tag => (0x7850aa, 0xc7a0f2),
        IconName::Film | IconName::Trash => (0xab455c, 0xf29bad),
        IconName::FileType => (0xb73541, 0xff8e98),
        IconName::Presentation => (0xad4b1e, 0xffab78),
        IconName::FileSpreadsheet => (0x207247, 0x7cdb9c),
        IconName::Table2 => (0x147984, 0x79d9e1),
        _ => (0x3667a3, 0x8cbcf5),
    };
    rgb(if mode.is_dark() { dark } else { light }).into()
}
