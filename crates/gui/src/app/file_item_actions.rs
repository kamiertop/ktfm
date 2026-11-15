use ely_core::menus::{Menu, MenuItem};
use ely_core::primitives::IconName;
use gpui_kit::{Context, MouseDownEvent};
use settings::config::{bookmark::BookmarkKind, tab::Location};
use vfs::EntryKind;

use super::App;

impl App {
    pub(super) fn open_file_item_menu(
        &mut self,
        generation: u64,
        index: usize,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        // 切换目录后虚拟列表会复用索引，过期的卡片事件不能作用到新目录。
        if self.browser.generation != generation {
            return;
        }
        let Some((path, kind)) = self.entry_target(index) else {
            return;
        };

        let owner = cx.weak_entity();
        let mut menu = Menu::new();
        if kind == EntryKind::Directory {
            let target = Location::directory(path.clone());
            let app = owner.clone();
            menu = menu.item(
                MenuItem::new("在新标签页打开")
                    .icon(IconName::FolderOpen)
                    .on_click(move |window, cx| {
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
            let label = if self.bookmarks.read(cx).find(&path).is_some() {
                "从书签移除"
            } else {
                "添加到书签"
            };
            let app = owner.clone();
            let target = path.clone();
            menu = menu.item(MenuItem::new(label).icon(IconName::Bookmark).on_click(
                move |_, cx| {
                    let _ = app.update(cx, |this, cx| {
                        this.toggle_bookmark(target.clone(), bookmark_kind, cx)
                    });
                },
            ));
        }

        for tag in self.tags(cx).items() {
            let id = tag.id;
            let app = owner.clone();
            let target = path.clone();
            menu = menu.item(
                MenuItem::check(format!("标签 · {}", tag.name), tag.contains(&path)).on_click(
                    move |_, cx| {
                        let _ =
                            app.update(cx, |this, cx| this.toggle_tag_path(id, target.clone(), cx));
                    },
                ),
            );
        }
        let menu = menu.item(
            MenuItem::new("新建标签并添加")
                .icon(IconName::Tag)
                .on_click(move |window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.edit_tag(None, Some(path.clone()), window, cx);
                    });
                }),
        );

        let menu = [
            ("剪切", IconName::Scissors),
            ("复制", IconName::Copy),
            ("重命名", IconName::PenLine),
            ("删除", IconName::Trash2),
        ]
        .into_iter()
        .fold(menu.separator(), |menu, (label, icon)| {
            menu.item(MenuItem::new(label).icon(icon).disabled(true))
        });
        self.file_item_menu.open(event.position, menu);
        cx.stop_propagation();
        cx.notify();
    }
}
