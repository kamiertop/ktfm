mod display;
mod input;
mod session;

use std::cell::Cell;
use std::path::PathBuf;

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use uuid::Uuid;

use settings::config::terminal::{
    DEFAULT_BACKGROUND, DEFAULT_COPY_SHORTCUT, DEFAULT_FONT_SIZE, DEFAULT_PASTE_SHORTCUT,
    FONT_SIZE_RANGE,
};

use super::{
    App,
    preferences::{ActivePreferences, parse_shortcut},
};
use crate::model::home_directory;
use display::Metrics;
use session::Session;

pub(super) fn parse_background(value: &str) -> Option<u32> {
    if value.len() != 7 || !value.starts_with('#') {
        return None;
    }
    let color = hex_color::HexColor::parse_rgb(value).ok()?;
    Some(u32::from_be_bytes([0, color.r, color.g, color.b]))
}

fn terminal_background(value: &str) -> u32 {
    parse_background(value.trim()).unwrap_or_else(|| {
        parse_background(DEFAULT_BACKGROUND).expect("默认终端背景色应为 #RRGGBB")
    })
}

const DEFAULT_HEIGHT: Pixels = px(264.);
const MIN_HEIGHT: Pixels = px(112.);
const HANDLE_HEIGHT: Pixels = px(6.);

pub(super) struct TabTerminal {
    view: Option<Entity<TerminalView>>,
    visible: bool,
    height: Pixels,
}

impl Default for TabTerminal {
    fn default() -> Self {
        Self {
            view: None,
            visible: false,
            height: DEFAULT_HEIGHT,
        }
    }
}

pub(super) struct TerminalResize {
    tab: Uuid,
    offset: Cell<Pixels>,
}

struct ResizePreview;

impl Render for ResizePreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl App {
    pub(super) fn toggle_embedded_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs[self.active_tab].terminal.visible {
            self.hide_embedded_terminal(window, cx);
        } else {
            self.show_embedded_terminal(window, cx);
        }
    }

    fn show_embedded_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self
            .location()
            .map(ToOwned::to_owned)
            .or_else(home_directory);
        let owner = cx.weak_entity();
        let terminal = &mut self.tabs[self.active_tab].terminal;
        if terminal.view.as_ref().is_none_or(|view| {
            view.read(cx)
                .session
                .as_ref()
                .is_none_or(|session| session.status.is_some())
        }) {
            terminal.view = Some(cx.new(|cx| TerminalView::new(directory, owner, window, cx)));
        }
        if let Some(view) = &terminal.view {
            window.focus(&view.read(cx).focus.clone(), cx);
        }
        terminal.visible = true;
        cx.notify();
    }

    fn hide_embedded_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs[self.active_tab].terminal.visible = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn resize_embedded_terminal(
        &mut self,
        event: &DragMoveEvent<TerminalResize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = &mut self.tabs[self.active_tab];
        let drag = event.drag(cx);
        if tab.id == drag.tab && tab.terminal.visible {
            tab.terminal.height = (event.bounds.bottom() - event.event.position.y
                + drag.offset.get())
            .clamp(MIN_HEIGHT, max_height(window));
            cx.notify();
        }
    }

    pub(super) fn embedded_terminal_panel(&self, window: &Window) -> Option<AnyElement> {
        let tab = &self.tabs[self.active_tab];
        let view = tab.terminal.view.clone().filter(|_| tab.terminal.visible)?;
        let height = tab.terminal.height.min(max_height(window));
        let drag = TerminalResize {
            tab: tab.id,
            offset: Cell::new(HANDLE_HEIGHT / 2.),
        };
        Some(
            v_flex()
                .h(height)
                .flex_shrink_0()
                .overflow_hidden()
                .child(
                    div()
                        .id("terminal-resize")
                        .h(HANDLE_HEIGHT)
                        .flex_shrink_0()
                        .bg(rgb(0x292f3b))
                        .hover(|style| style.bg(rgb(0x6590cf)))
                        .cursor(CursorStyle::ResizeUpDown)
                        .on_drag(drag, |drag, offset, _, cx| {
                            drag.offset.set(offset.y.clamp(px(0.), HANDLE_HEIGHT));
                            cx.new(|_| ResizePreview)
                        }),
                )
                .child(view)
                .into_any_element(),
        )
    }
}

fn max_height(window: &Window) -> Pixels {
    let height = window.viewport_size().height;
    (height * 0.75).min(height - px(180.)).max(MIN_HEIGHT)
}

struct TerminalView {
    owner: WeakEntity<App>,
    focus: FocusHandle,
    session: Option<Session>,
    error: Option<String>,
    metrics: Option<Metrics>,
    font: Font,
    selecting: bool,
    composing: String,
    scroll_remainder: f32,
}

impl TerminalView {
    fn new(
        directory: Option<PathBuf>,
        owner: WeakEntity<App>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        cx.on_focus_out(&focus, window, |view, _, _, cx| {
            view.composing.clear();
            view.selecting = false;
            cx.notify();
        })
        .detach();
        cx.observe_global::<ActivePreferences>(|view, cx| {
            if let Some(session) = &mut view.session {
                let config = &cx.global::<ActivePreferences>().0.terminal;
                session.set_cursor_shape(config.cursor_shape);
            }
            cx.notify();
        })
        .detach();
        let (session, error) = match Session::start(directory, cx) {
            Ok(session) => (Some(session), None),
            Err(error) => (None, Some(format!("无法启动内置终端：{error:#}"))),
        };
        Self {
            owner,
            focus,
            session,
            error,
            metrics: None,
            font: display::terminal_font(cx),
            selecting: false,
            composing: String::new(),
            scroll_remainder: 0.,
        }
    }

    fn select(
        &mut self,
        position: Point<Pixels>,
        kind: Option<SelectionType>,
        cx: &mut Context<Self>,
    ) {
        let (Some(metrics), Some(session)) = (&self.metrics, &self.session) else {
            return;
        };
        let local = position - metrics.bounds.origin;
        let column =
            (local.x / metrics.cell.width).clamp(0., f32::from(metrics.size.columns) - 0.01);
        let row = (local.y / metrics.cell.height).clamp(0., f32::from(metrics.size.rows) - 1.);
        let side = if column.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        let mut term = session.grid.lock();
        let point = GridPoint::new(
            Line(row.floor() as i32 - term.grid().display_offset() as i32),
            Column(column.floor() as usize),
        );
        if let Some(kind) = kind {
            term.selection = Some(Selection::new(kind, point, side));
        } else if let Some(selection) = &mut term.selection {
            selection.update(point, side);
        }
        cx.notify();
    }

    fn send_input(&mut self, bytes: impl Into<Vec<u8>>, cx: &mut Context<Self>) {
        if let Some(session) = self
            .session
            .as_ref()
            .filter(|session| session.status.is_none())
        {
            let mut term = session.grid.lock();
            term.selection = None;
            term.scroll_display(Scroll::Bottom);
            drop(term);
            session.write(bytes);
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let modifiers = key.modifiers;
        let Some(session) = &self.session else { return };
        let config = &cx.global::<ActivePreferences>().0.terminal;
        if modifiers.control || modifiers.alt || modifiers.platform {
            if matches_shortcut(key, &config.copy_shortcut, DEFAULT_COPY_SHORTCUT) {
                let text = session.grid.lock().selection_to_string();
                if let Some(text) = text.filter(|text| !text.is_empty()) {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    cx.stop_propagation();
                    return;
                }
            } else if matches_shortcut(key, &config.paste_shortcut, DEFAULT_PASTE_SHORTCUT) {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    let mode = *session.grid.lock().mode();
                    let text = if mode.contains(TermMode::BRACKETED_PASTE) {
                        format!(
                            "\x1b[200~{}\x1b[201~",
                            text.replace(['\x1b', '\u{009b}'], "")
                        )
                    } else {
                        text.replace("\r\n", "\r").replace('\n', "\r")
                    };
                    self.send_input(text, cx);
                }
                cx.stop_propagation();
                return;
            }
        }
        if modifiers.platform || modifiers.function {
            return;
        }
        // 普通文本走系统输入处理器，控制键与快捷键由终端编码。
        let plain_text = modifiers.is_subset_of(&Modifiers::shift())
            && key
                .key_char
                .as_ref()
                .is_some_and(|text| !text.chars().any(char::is_control))
            && !matches!(
                key.key.as_str(),
                "enter" | "tab" | "backspace" | "escape" | "delete"
            );
        if plain_text {
            return;
        }
        let mode = *session.grid.lock().mode();
        if let Some(input) = terminal_key(key, mode) {
            self.send_input(input, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let config = &cx.global::<ActivePreferences>().0.terminal;
        let background = terminal_background(&config.background);
        let font_size = config.font_size;
        let font_size = px(if font_size.is_finite() {
            font_size.clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end())
        } else {
            DEFAULT_FONT_SIZE
        });
        let font = self.font.clone();
        let prepaint = entity.downgrade();
        let paint = prepaint.clone();
        let status = self
            .error
            .clone()
            .or_else(|| self.session.as_ref()?.status.clone());
        let body = div()
            .id("embedded-terminal")
            .key_context("EmbeddedTerminal")
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .bg(rgb(background))
            .px_2()
            .py_1()
            .overflow_hidden()
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.selecting = true;
                    let kind = match event.click_count {
                        1 => SelectionType::Simple,
                        2 => SelectionType::Semantic,
                        _ => SelectionType::Lines,
                    };
                    this.select(event.position, Some(kind), cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.selecting && event.pressed_button == Some(MouseButton::Left) {
                    this.select(event.position, None, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.selecting = false),
            )
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                if let (Some(session), Some(metrics)) = (&this.session, &this.metrics) {
                    this.scroll_remainder += match event.delta {
                        ScrollDelta::Lines(delta) => delta.y * 3.,
                        ScrollDelta::Pixels(delta) => delta.y / metrics.cell.height,
                    };
                    let lines = this.scroll_remainder as i32;
                    this.scroll_remainder -= lines as f32;
                    if lines != 0 {
                        session.grid.lock().scroll_display(Scroll::Delta(lines));
                        cx.notify();
                    }
                    cx.stop_propagation();
                }
            }))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let _ = prepaint.update(cx, |view, _| {
                            let metrics = Metrics::new(bounds, font, font_size, background, window);
                            if let Some(session) = &mut view.session {
                                session.resize(metrics.size);
                            }
                            view.metrics = Some(metrics);
                        });
                    },
                    move |_, (), window, cx| {
                        let Some(view) = paint.upgrade() else { return };
                        let view = view.read(cx);
                        if let (Some(metrics), Some(session)) = (&view.metrics, &view.session) {
                            let metrics = metrics.clone();
                            let grid = session.grid.clone();
                            let running = session.status.is_none();
                            let focus = view.focus.clone();
                            let focused = focus.is_focused(window);
                            let composing = if focused {
                                view.composing.clone()
                            } else {
                                String::new()
                            };
                            if running && focused {
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(metrics.bounds, entity),
                                    cx,
                                );
                            }
                            metrics.paint(&grid, running, focused, &composing, window, cx);
                        }
                    },
                )
                .size_full(),
            )
            .when_some(status, |panel, status| {
                panel.relative().child(
                    div()
                        .absolute()
                        .right_2()
                        .bottom_1()
                        .px_2()
                        .rounded_sm()
                        .bg(rgb(0x292f3b))
                        .text_sm()
                        .text_color(rgb(0xe6e9ef))
                        .child(status),
                )
            });
        let title = self.session.as_ref().map_or_else(
            || SharedString::from("终端"),
            |session| session.title.clone(),
        );
        let owner = self.owner.clone();
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .h(px(28.))
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .bg(rgb(0x202530))
                    .text_color(rgb(0xd1d8e4))
                    .child(div().min_w_0().truncate().text_sm().child(title))
                    .child(
                        div()
                            .id("hide-terminal")
                            .flex_shrink_0()
                            .cursor_pointer()
                            .p_1()
                            .rounded_sm()
                            .hover(|style| style.bg(rgb(0x353d4d)))
                            .child(Icon::new(IconName::X).size(px(14.)))
                            .on_click(move |_, window, cx| {
                                let _ = owner
                                    .update(cx, |app, cx| app.hide_embedded_terminal(window, cx));
                            }),
                    ),
            )
            .child(body)
    }
}

fn matches_shortcut(key: &Keystroke, value: &str, default: &str) -> bool {
    parse_shortcut(value.trim(), "内置终端")
        .or_else(|_| parse_shortcut(default, "内置终端"))
        .is_ok_and(|binding| key.modifiers == binding.modifiers && key.key == binding.key)
}

fn terminal_key(key: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let modifiers = key.modifiers;
    let name = key.key.as_str();
    let modifier_code = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let cursor = match name {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    };
    if let Some(cursor) = cursor {
        return Some(
            if modifier_code > 1 {
                format!("\x1b[1;{modifier_code}{cursor}")
            } else if mode.contains(TermMode::APP_CURSOR) {
                format!("\x1bO{cursor}")
            } else {
                format!("\x1b[{cursor}")
            }
            .into_bytes(),
        );
    }
    if let Some(number) = match name {
        "insert" => Some(2),
        "delete" => Some(3),
        "pageup" => Some(5),
        "pagedown" => Some(6),
        "f5" => Some(15),
        "f6" => Some(17),
        "f7" => Some(18),
        "f8" => Some(19),
        "f9" => Some(20),
        "f10" => Some(21),
        "f11" => Some(23),
        "f12" => Some(24),
        _ => None,
    } {
        return Some(
            if modifier_code > 1 {
                format!("\x1b[{number};{modifier_code}~")
            } else {
                format!("\x1b[{number}~")
            }
            .into_bytes(),
        );
    }
    if let Some(function) = match name {
        "f1" => Some('P'),
        "f2" => Some('Q'),
        "f3" => Some('R'),
        "f4" => Some('S'),
        _ => None,
    } {
        return Some(
            if modifier_code > 1 {
                format!("\x1b[1;{modifier_code}{function}")
            } else {
                format!("\x1bO{function}")
            }
            .into_bytes(),
        );
    }
    let mut bytes = match name {
        "enter" => vec![b'\r'],
        "escape" => vec![0x1b],
        "backspace" => vec![if modifiers.control { 8 } else { 127 }],
        "tab" if modifiers.shift => b"\x1b[Z".to_vec(),
        "tab" => vec![b'\t'],
        _ if modifiers.control => match name.as_bytes() {
            [letter @ b'a'..=b'z'] => vec![letter & 0x1f],
            [b'@' | b' '] | b"space" => vec![0],
            [letter @ (b'[' | b'\\' | b']' | b'^' | b'_')] => vec![letter & 0x1f],
            [b'?'] => vec![127],
            _ => return None,
        },
        _ => key.key_char.as_ref()?.as_bytes().to_vec(),
    };
    if modifiers.alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_COPY_SHORTCUT, DEFAULT_PASTE_SHORTCUT, matches_shortcut, terminal_key};
    use alacritty_terminal::term::TermMode;
    use gpui_kit::Keystroke;

    #[test]
    fn clipboard_bindings_can_be_reassigned_and_control_c_encodes_interrupt() {
        let control_c = Keystroke::parse("ctrl-c").unwrap();
        let control_v = Keystroke::parse("ctrl-v").unwrap();
        let alt_c = Keystroke::parse("alt-c").unwrap();
        assert!(matches_shortcut(
            &control_c,
            "ctrl-c",
            DEFAULT_COPY_SHORTCUT
        ));
        assert!(matches_shortcut(
            &control_v,
            "ctrl-v",
            DEFAULT_PASTE_SHORTCUT
        ));
        assert!(matches_shortcut(&alt_c, "alt-c", DEFAULT_COPY_SHORTCUT));
        assert!(!matches_shortcut(
            &control_c,
            "alt-c",
            DEFAULT_COPY_SHORTCUT
        ));
        assert_eq!(terminal_key(&control_c, TermMode::empty()), Some(vec![3]));
    }
}
