use std::sync::OnceLock;

use alacritty_terminal::Term;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use super::session::{PtySize, TerminalEvents};

const FOREGROUND: u32 = 0xe6e9ef;
const SELECTION: u32 = 0x325484;
const ANSI: [u32; 16] = [
    0x1c2028, 0xf47880, 0x8ed69c, 0xeac97d, 0x82adf2, 0xc699ee, 0x77d1d8, 0xc9ced9, 0x687386,
    0xff989d, 0xa7e6b3, 0xffda95, 0x9ec0ff, 0xddb4ff, 0xa0e7eb, 0xf2f4f8,
];

struct BackgroundRun {
    row: i32,
    start: usize,
    end: usize,
    color: Hsla,
}

pub(super) fn terminal_font(cx: &App) -> Font {
    static FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
    let fallback = &cx.theme().mono_font_family;
    let families = FAMILIES.get_or_init(|| cx.text_system().all_font_names());
    font(preferred_font(fallback.as_ref(), families).to_owned())
}

fn preferred_font<'a>(fallback: &'a str, families: &'a [String]) -> &'a str {
    let is_nerd_mono = |name: &str| {
        name.ends_with(" Nerd Font Mono") || name.ends_with(" NF CN") || name.ends_with(" NFM")
    };
    if is_nerd_mono(fallback) {
        return fallback;
    }
    families
        .iter()
        .find(|name| name.ends_with(" NF CN"))
        .or_else(|| families.iter().find(|name| is_nerd_mono(name)))
        .map_or(fallback, String::as_str)
}

#[derive(Clone)]
pub(super) struct Metrics {
    pub bounds: Bounds<Pixels>,
    pub cell: Size<Pixels>,
    pub size: PtySize,
    font: Font,
    font_size: Pixels,
    background: u32,
}

impl Metrics {
    pub fn new(
        bounds: Bounds<Pixels>,
        font: Font,
        font_size: Pixels,
        background: u32,
        window: &Window,
    ) -> Self {
        let text = window.text_system();
        let width = text
            .ch_advance(text.resolve_font(&font), font_size)
            .unwrap_or(font_size * 0.6)
            .max(px(1.));
        let height = (font_size * 1.4).ceil();
        let columns = (bounds.size.width / width).floor() as u16;
        let rows = (bounds.size.height / height).floor() as u16;
        Self {
            bounds,
            cell: size(width, height),
            size: PtySize {
                columns: columns.max(2),
                rows: rows.max(1),
                cell_width: f32::from(width).round() as u16,
                cell_height: f32::from(height) as u16,
            },
            font,
            font_size,
            background,
        }
    }

    pub(super) fn origin(&self, column: usize, row: i32) -> Point<Pixels> {
        self.bounds.origin + point(self.cell.width * column, self.cell.height * row as f32)
    }

    pub fn paint(
        &self,
        grid: &FairMutex<Term<TerminalEvents>>,
        running: bool,
        focused: bool,
        composing: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let term = grid.lock();
        let content = term.renderable_content();
        let cursor = content.cursor;
        let cursor_row = cursor.point.line.0 + content.display_offset as i32;
        let cursor_origin = self.origin(cursor.point.column.0, cursor_row);
        let terminal_color = |value| color(value, content.colors, self.background);
        let cursor_color = terminal_color(Color::Named(NamedColor::Cursor));
        let base_background = terminal_color(Color::Named(NamedColor::Background));
        let base_foreground = terminal_color(Color::Named(NamedColor::Foreground));

        let mut cells = Vec::new();
        let mut backgrounds: Vec<BackgroundRun> = Vec::new();
        for indexed in content.display_iter {
            let cell = indexed.cell;
            let location = indexed.point;
            let flags = cell.flags;
            let mut foreground = terminal_color(cell.fg);
            let mut background = terminal_color(cell.bg);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut foreground, &mut background);
            }
            if content
                .selection
                .is_some_and(|selection| selection.contains(location))
            {
                background = rgb(SELECTION).into();
            }
            let under_cursor = running
                && focused
                && content.display_offset == 0
                && composing.is_empty()
                && cursor.shape == CursorShape::Block
                && location == cursor.point;
            if under_cursor {
                background = cursor_color;
                foreground = base_background;
            }
            let column = location.column.0;
            let row = location.line.0 + content.display_offset as i32;
            if background != base_background {
                // 同色相邻格共用一个矩形，避免逐格对齐设备像素时露出底色。
                if let Some(run) = backgrounds.last_mut()
                    && run.row == row
                    && run.end == column
                    && run.color == background
                {
                    run.end += 1;
                } else {
                    backgrounds.push(BackgroundRun {
                        row,
                        start: column,
                        end: column + 1,
                        color: background,
                    });
                }
            }
            let mut text = String::new();
            let spacer =
                flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
            if !spacer && !flags.contains(Flags::HIDDEN) {
                if cell.c != ' ' || flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT) {
                    text.push(cell.c);
                }
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra);
                }
            }
            if !text.is_empty() {
                cells.push((self.origin(column, row), foreground, text, flags));
            }
        }
        let show_cursor = running && content.display_offset == 0;
        drop(term);

        if base_background != rgb(self.background).into() {
            window.paint_quad(fill(self.bounds, base_background));
        }
        for run in backgrounds {
            let bounds = Bounds::from_corners(
                self.origin(run.start, run.row),
                self.origin(run.end, run.row + 1),
            );
            window.paint_quad(fill(bounds, run.color));
        }
        for (origin, foreground, text, flags) in cells {
            if let Some(path) = powerline_path(&text, Bounds::new(origin, self.cell)) {
                window.paint_path(path, foreground);
                continue;
            }
            let mut font = self.font.clone();
            if flags.contains(Flags::BOLD) {
                font.weight = FontWeight::BOLD;
            }
            if flags.contains(Flags::ITALIC) {
                font.style = FontStyle::Italic;
            }
            let run = TextRun {
                len: text.len(),
                font,
                color: foreground,
                background_color: None,
                underline: flags
                    .intersects(Flags::ALL_UNDERLINES)
                    .then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(foreground),
                        wavy: flags.contains(Flags::UNDERCURL),
                    }),
                strikethrough: flags
                    .contains(Flags::STRIKEOUT)
                    .then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(foreground),
                    }),
            };
            let line = window
                .text_system()
                .shape_line(text.into(), self.font_size, &[run], None);
            let _ = line.paint(origin, self.cell.height, TextAlign::Left, None, window, cx);
        }

        if show_cursor && !composing.is_empty() {
            let foreground = base_foreground;
            let run = TextRun {
                len: composing.len(),
                font: self.font.clone(),
                color: foreground,
                background_color: Some(base_background),
                underline: Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(foreground),
                    wavy: false,
                }),
                strikethrough: None,
            };
            let line = window.text_system().shape_line(
                composing.to_owned().into(),
                self.font_size,
                &[run],
                None,
            );
            let _ = line.paint(
                cursor_origin,
                self.cell.height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
            return;
        }
        if show_cursor {
            let cursor_size = match cursor.shape {
                CursorShape::Beam if focused => size(px(2.), self.cell.height),
                CursorShape::Underline if focused => size(self.cell.width, px(2.)),
                CursorShape::Block
                | CursorShape::HollowBlock
                | CursorShape::Beam
                | CursorShape::Underline
                    if !focused || cursor.shape == CursorShape::HollowBlock =>
                {
                    let bounds = Bounds::new(cursor_origin, self.cell);
                    window.paint_quad(outline(bounds, cursor_color));
                    return;
                }
                _ => return,
            };
            let origin = if cursor.shape == CursorShape::Underline {
                cursor_origin + point(px(0.), self.cell.height - px(2.))
            } else {
                cursor_origin
            };
            window.paint_quad(fill(Bounds::new(origin, cursor_size), cursor_color));
        }
    }
}

// Powerline 的实心分隔符贴合整个单元格，背景色带不会受字体基线或字宽影响。
fn powerline_path(text: &str, bounds: Bounds<Pixels>) -> Option<Path<Pixels>> {
    let (mirrored, rounded) = match text {
        "\u{e0b0}" => (false, false),
        "\u{e0b2}" => (true, false),
        "\u{e0b4}" => (false, true),
        "\u{e0b6}" => (true, true),
        _ => return None,
    };

    let at = |x, y: f32| {
        let x = if mirrored { 1. - x } else { x };
        bounds.origin + point(bounds.size.width * x, bounds.size.height * y)
    };
    let mut path = PathBuilder::fill();
    path.move_to(at(0., 0.));
    if rounded {
        const K: f32 = 0.552_284_8;
        path.cubic_bezier_to(at(1., 0.5), at(K, 0.), at(1., 0.5 * (1. - K)));
        path.cubic_bezier_to(at(0., 1.), at(1., 0.5 * (1. + K)), at(K, 1.));
    } else {
        path.line_to(at(1., 0.5));
        path.line_to(at(0., 1.));
    }
    path.close();
    path.build().ok()
}

fn outline(bounds: Bounds<Pixels>, color: Hsla) -> PaintQuad {
    quad(
        bounds,
        px(0.),
        transparent_black(),
        px(1.),
        color,
        BorderStyle::Solid,
    )
}

fn color(color: Color, colors: &Colors, background: u32) -> Hsla {
    let value = match color {
        Color::Spec(value) => value,
        Color::Named(value) => palette(value as usize, colors, background),
        Color::Indexed(value) => palette(value as usize, colors, background),
    };
    rgb((u32::from(value.r) << 16) | (u32::from(value.g) << 8) | u32::from(value.b)).into()
}

pub(super) fn palette(index: usize, colors: &Colors, background: u32) -> Rgb {
    if index < alacritty_terminal::term::color::COUNT
        && let Some(color) = colors[index]
    {
        return color;
    }
    let value = match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let level = |index| [0u32, 95, 135, 175, 215, 255][index];
            let index = index - 16;
            (level(index / 36) << 16) | (level(index / 6 % 6) << 8) | level(index % 6)
        }
        232..=255 => ((index as u32 - 232) * 10 + 8) * 0x010101,
        257 => background,
        259..=266 => ANSI[index - 259],
        _ => {
            let Rgb { r, g, b } = palette(NamedColor::Background as usize, colors, background);
            let brightness = 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b);
            if brightness >= 140_000 {
                0x161a22
            } else {
                FOREGROUND
            }
        }
    };
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::{Colors, NamedColor, palette, powerline_path, preferred_font};

    #[test]
    fn light_background_switches_default_text_and_preserves_ansi_colors() {
        let colors = Colors::default();
        let foreground = NamedColor::Foreground as usize;
        let dark = palette(foreground, &colors, 0x161a22);
        let light = palette(foreground, &colors, 0xfafafa);
        assert!(dark.r > light.r);
        assert_eq!(palette(1, &colors, 0x161a22), palette(1, &colors, 0xfafafa));
        let background = palette(NamedColor::Background as usize, &colors, 0xfafafa);
        assert_eq!([background.r, background.g, background.b], [250; 3]);
    }
    use gpui_kit::{Bounds, point, px, size};

    #[test]
    fn prefers_patched_monospace_and_keeps_an_existing_nerd_font() {
        let families = vec![
            "JetBrainsMono Nerd Font Mono".to_owned(),
            "Maple Mono Normal NL NF CN".to_owned(),
        ];
        assert_eq!(
            preferred_font("Adwaita Mono", &families),
            "Maple Mono Normal NL NF CN",
        );
        assert_eq!(
            preferred_font("JetBrainsMono Nerd Font Mono", &families),
            "JetBrainsMono Nerd Font Mono",
        );
        assert_eq!(preferred_font("Adwaita Mono", &[]), "Adwaita Mono");
    }

    #[test]
    fn powerline_separators_fill_the_cell_from_edge_to_edge() {
        let cell = Bounds::new(point(px(8.), px(12.)), size(px(8.), px(18.)));
        for glyph in ["\u{e0b0}", "\u{e0b2}", "\u{e0b4}", "\u{e0b6}"] {
            assert_eq!(powerline_path(glyph, cell).unwrap().bounds, cell);
        }
        assert!(powerline_path("\u{f43a}", cell).is_none());
    }
}
