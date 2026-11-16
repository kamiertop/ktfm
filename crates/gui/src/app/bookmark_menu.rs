use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon,
    menu::{PopupMenu, PopupMenuItem},
};
use gpui_kit::{Context, Pixels, WeakEntity, Window, px};
use settings::config::{bookmark::BookmarkKind, tab::Location};
use uuid::Uuid;

use super::App;

// 保持书签菜单紧凑，较长的菜单项可以撑宽。
const BOOKMARK_MENU_MIN_WIDTH: Pixels = px(140.);

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
            Self::Remove => ("从书签移除", Some(IconName::Trash)),
            Self::Rename => ("重命名", Some(IconName::PenLine)),
            Self::Properties => ("属性", Some(IconName::Info)),
        }
    }
}

impl App {
    pub(super) fn build_bookmark_menu(
        &self,
        id: Uuid,
        anchor_y: Pixels,
        mut menu: PopupMenu,
        owner: &WeakEntity<Self>,
        cx: &gpui_kit::App,
    ) -> PopupMenu {
        let Some(bookmark) = self.bookmarks.read(cx).get(id) else {
            return menu;
        };
        let has_directory =
            bookmark.kind == BookmarkKind::Directory || bookmark.path.0.parent().is_some();
        menu = menu.min_w(BOOKMARK_MENU_MIN_WIDTH);
        for command in COMMANDS {
            if matches!(command, Command::Remove) {
                menu = menu.separator();
            }
            let (label, icon) = command.appearance(bookmark.kind);
            let disabled =
                !has_directory && matches!(command, Command::NewTab | Command::NewWindow);
            let mut item = PopupMenuItem::new(label).disabled(disabled);
            if let Some(icon) = icon {
                item = item.icon(Icon::new(icon));
            }
            let owner = owner.clone();
            menu = menu.item(item.on_click(move |_, window, cx| {
                let owner = owner.clone();
                // 先让菜单退出并还原焦点，再打开需要接管焦点的编辑框。
                window.defer(cx, move |window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.apply_bookmark_command(id, command, anchor_y, window, cx);
                    });
                });
            }));
        }
        menu
    }

    fn apply_bookmark_command(
        &mut self,
        id: Uuid,
        command: Command,
        anchor_y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                self.change_bookmarks(move |bookmarks| bookmarks.remove(id), cx);
            }
            Command::Rename => {
                self.rename_bookmark(id, anchor_y, window, cx);
            }
            Command::Properties => {
                self.open_bookmark_properties(id, window, cx);
            }
        }
    }
}
