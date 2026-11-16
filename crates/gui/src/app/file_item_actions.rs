use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon,
    menu::{PopupMenu, PopupMenuItem},
};
use gpui_kit::{Context, WeakEntity, Window};
use settings::command::AppCommand;
use settings::config::{bookmark::BookmarkKind, tab::Location};
use vfs::EntryKind;

use super::App;
use super::preferences::ActivePreferences;

impl App {
    pub(super) fn middle_click_folder(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, EntryKind::Directory)) = self.entry_target(index) else {
            return;
        };
        match cx
            .global::<ActivePreferences>()
            .0
            .actions
            .middle_click_folder
            .to_command(&path)
        {
            Some(AppCommand::CreateNewTab(path)) => {
                self.new_tab(Some(Location::directory(path)), window, cx);
            }
            Some(AppCommand::OpenNewWindow(path)) => {
                if let Err(error) = Self::open_window_at(cx, Some((path, self.bookmarks.clone()))) {
                    self.workspace_error = Some(format!("无法打开新窗口：{error}"));
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    pub(super) fn build_file_item_menu(
        &self,
        snapshot_revision: u64,
        index: usize,
        mut menu: PopupMenu,
        owner: &WeakEntity<Self>,
        cx: &gpui_kit::App,
    ) -> PopupMenu {
        // 切换目录后虚拟列表会复用索引，过期的卡片事件不能作用到新目录。
        if self.browser.snapshot_revision != snapshot_revision {
            return menu;
        }
        let Some((path, kind)) = self.entry_target(index) else {
            return menu;
        };

        if kind == EntryKind::Directory {
            let target = Location::directory(path.clone());
            let app = owner.clone();
            menu = menu.item(
                PopupMenuItem::new("在新标签页打开")
                    .icon(Icon::new(IconName::FolderOpen))
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.new_tab(Some(target.clone()), window, cx);
                        });
                    }),
            );
        }
        let bookmark_kind = match kind {
            EntryKind::Directory => Some(BookmarkKind::Directory),
            EntryKind::File | EntryKind::Symlink => Some(BookmarkKind::File),
            _ => None,
        };
        if let Some(bookmark_kind) = bookmark_kind
            && !self.is_default_favorite(&path)
        {
            let bookmarked = self.bookmarks.read(cx).find(&path).is_some();
            let label = if bookmarked {
                "从书签移除"
            } else {
                "添加到书签"
            };
            let app = owner.clone();
            let target = path.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .icon(Icon::new(IconName::Bookmark))
                    .on_click(move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.set_bookmark(target.clone(), bookmark_kind, !bookmarked, cx)
                        });
                    }),
            );
        }

        for tag in self.tags(cx).items() {
            let id = tag.id;
            let app = owner.clone();
            let target = path.clone();
            let checked = tag.contains(&path);
            menu = menu.item(
                PopupMenuItem::new(format!("标签 · {}", tag.name))
                    .checked(checked)
                    .on_click(move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.set_tag_path(id, target.clone(), !checked, cx)
                        });
                    }),
            );
        }
        let owner = owner.clone();
        let menu = menu.item(
            PopupMenuItem::new("新建标签并添加")
                .icon(Icon::new(IconName::Tag))
                .on_click(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.edit_tag(None, Some(path.clone()), window, cx);
                    });
                }),
        );

        [
            ("剪切", IconName::Scissors),
            ("复制", IconName::Copy),
            ("重命名", IconName::PenLine),
            ("删除", IconName::Trash),
        ]
        .into_iter()
        .fold(menu.separator(), |menu, (label, icon)| {
            menu.item(
                PopupMenuItem::new(label)
                    .icon(Icon::new(icon))
                    .disabled(true),
            )
        })
    }
}
