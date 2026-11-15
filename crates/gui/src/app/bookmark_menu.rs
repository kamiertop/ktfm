use ely_core::forms::surface;
use ely_core::primitives::{Icon, IconName, raise};
use ely_core::theme::{ActiveTheme, ControlSize, IconSize, Radius};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{bookmark::BookmarkKind, tab::Location};
use uuid::Uuid;

use super::App;

// 只控制书签菜单的最小宽度，较长的菜单项可以把面板撑开。
const BOOKMARK_MENU_MIN_WIDTH: Pixels = px(140.);

pub(super) struct BookmarkMenu {
    id: Uuid,
    position: Point<Pixels>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    selected: usize,
}

#[derive(Clone, Copy)]
enum Command {
    Open,
    NewTab,
    NewWindow,
    Remove,
    Rename,
    Properties,
}

const COMMANDS: [Command; 6] = [
    Command::Open,
    Command::NewTab,
    Command::NewWindow,
    Command::Remove,
    Command::Rename,
    Command::Properties,
];

impl Command {
    fn appearance(self, kind: BookmarkKind) -> (&'static str, Option<IconName>) {
        match self {
            Self::Open => ("打开", Some(IconName::FolderOpen)),
            Self::NewTab if kind == BookmarkKind::File => ("在新标签页打开所在目录", None),
            Self::NewWindow if kind == BookmarkKind::File => ("在新窗口打开所在目录", None),
            Self::NewTab => ("在新标签页打开", None),
            Self::NewWindow => ("在新窗口打开", None),
            Self::Remove => ("从书签移除", Some(IconName::Trash2)),
            Self::Rename => ("重命名", Some(IconName::PenLine)),
            Self::Properties => ("属性", Some(IconName::Info)),
        }
    }
}

impl App {
    pub(super) fn open_bookmark_menu(
        &mut self,
        id: Uuid,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_item_menu.clear();
        self.close_bookmark_menu(window, cx);
        let focus = cx.focus_handle();
        let previous_focus = window.focused(cx);
        window.focus(&focus, cx);
        self.bookmark_menu = Some(BookmarkMenu {
            id,
            position,
            focus,
            previous_focus,
            selected: 0,
        });
        cx.notify();
    }

    fn close_bookmark_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.bookmark_menu.take() {
            if let Some(focus) = menu.previous_focus {
                window.focus(&focus, cx);
            }
            cx.notify();
        }
    }

    fn apply_bookmark_command(
        &mut self,
        id: Uuid,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let anchor_y = self.bookmark_menu.as_ref().map(|menu| menu.position.y);
        self.close_bookmark_menu(window, cx);
        match command {
            Command::Open => self.open_bookmark(id, window, cx),
            Command::NewTab | Command::NewWindow => {
                let Some(bookmark) = self.bookmarks.read(cx).get(id) else {
                    return;
                };
                let path = if bookmark.kind == BookmarkKind::File {
                    bookmark.path.0.parent()
                } else {
                    Some(bookmark.path.0.as_path())
                };
                let Some(path) = path.map(ToOwned::to_owned) else {
                    return;
                };
                if matches!(command, Command::NewTab) {
                    self.new_tab(Some(Location::directory(path)), window, cx);
                } else if let Err(error) =
                    Self::open_window_at(cx, Some((path, self.bookmarks.clone())))
                {
                    self.workspace_error = Some(format!("无法打开新窗口：{error}"));
                    cx.notify();
                }
            }
            Command::Remove => {
                self.change_bookmarks(|bookmarks| bookmarks.remove(id), cx);
            }
            Command::Rename => {
                if let Some(anchor_y) = anchor_y {
                    self.rename_bookmark(id, anchor_y, window, cx);
                }
            }
            Command::Properties => {
                self.bookmark_properties = Some(id);
                cx.notify();
            }
        }
    }

    pub(super) fn bookmark_menu_view(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let menu = self.bookmark_menu.as_ref()?;
        let bookmark = self.bookmarks.read(cx).get(menu.id)?;
        let kind = bookmark.kind;
        let id = menu.id;
        let position = menu.position;
        let selected = menu.selected;
        let focus = menu.focus.clone();
        let has_directory = kind == BookmarkKind::Directory || bookmark.path.0.parent().is_some();
        let theme = cx.theme();
        let (fg, muted, disabled_fg, hover, border) = (
            theme.colors.fg,
            theme.colors.fg_muted,
            theme.colors.fg_disabled,
            theme.colors.hover,
            theme.colors.border,
        );
        let row_height = theme.control_height(ControlSize::Md);
        let icon_size = theme.icon_size(IconSize::Sm);
        let radius = theme.radius(Radius::Md);

        let panel = surface("bookmark-menu", cx)
            .track_focus(&focus)
            .min_w(BOOKMARK_MENU_MIN_WIDTH)
            .p_1()
            .flex()
            .flex_col()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                this.close_bookmark_menu(window, cx);
            }))
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "escape" => this.close_bookmark_menu(window, cx),
                    "enter" | "space" => {
                        if let Some(menu) = &this.bookmark_menu {
                            this.apply_bookmark_command(id, COMMANDS[menu.selected], window, cx);
                        }
                    }
                    direction @ ("up" | "down") => {
                        let Some(menu) = this.bookmark_menu.as_mut() else {
                            return;
                        };
                        loop {
                            menu.selected = if direction == "down" {
                                (menu.selected + 1) % COMMANDS.len()
                            } else {
                                (menu.selected + COMMANDS.len() - 1) % COMMANDS.len()
                            };
                            if has_directory
                                || !matches!(
                                    COMMANDS[menu.selected],
                                    Command::NewTab | Command::NewWindow
                                )
                            {
                                break;
                            }
                        }
                        cx.notify();
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .children(
                COMMANDS
                    .into_iter()
                    .enumerate()
                    .flat_map(|(index, command)| {
                        let (label, icon) = command.appearance(kind);
                        let enabled = has_directory
                            || !matches!(command, Command::NewTab | Command::NewWindow);
                        let separator = matches!(command, Command::Remove)
                            .then(|| div().h_px().my_1().bg(border).into_any_element());
                        let row = div()
                            .id(("bookmark-menu-row", index))
                            .flex()
                            .items_center()
                            .h(row_height)
                            .gap_2()
                            .px_2()
                            .rounded(radius)
                            .text_color(if enabled { fg } else { disabled_fg })
                            .when(enabled, |row| {
                                row.cursor_pointer()
                                    .when(index == selected, |row| row.bg(hover))
                                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                        if let Some(menu) = this.bookmark_menu.as_mut()
                                            && menu.selected != index
                                        {
                                            menu.selected = index;
                                            cx.notify();
                                        }
                                    }))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, window, cx| {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            this.apply_bookmark_command(id, command, window, cx);
                                        }),
                                    )
                            })
                            .child(div().size(icon_size).flex_none().when_some(
                                icon,
                                |slot, icon| {
                                    slot.child(Icon::new(icon).size(IconSize::Sm).color(muted))
                                },
                            ))
                            .child(div().flex_none().whitespace_nowrap().child(label));
                        separator.into_iter().chain([row.into_any_element()])
                    }),
            );

        Some(raise(
            "bookmark-menu-layer",
            anchored().position(position).snap_to_window().child(panel),
        ))
    }
}
