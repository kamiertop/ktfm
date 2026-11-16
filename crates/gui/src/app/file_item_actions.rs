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
use super::browser::DirectoryState;
use super::file_operations::FileAction;
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
        if kind == EntryKind::Directory {
            menu = menu.item(external_terminal_item(path.clone(), owner));
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
        let tag_owner = owner.clone();
        let menu = menu.item(
            PopupMenuItem::new("新建标签并添加")
                .icon(Icon::new(IconName::Tag))
                .on_click(move |_, window, cx| {
                    let _ = tag_owner.update(cx, |this, cx| {
                        this.edit_tag(None, Some(path.clone()), window, cx);
                    });
                }),
        );

        let unavailable = self.file_job_running || kind == EntryKind::Missing;
        let multiple = self.browser.selection.contains(&index) && self.browser.selection.len() > 1;
        let target = Some((snapshot_revision, index));
        [
            ("剪切", IconName::Scissors, FileAction::Cut, unavailable),
            ("复制", IconName::Copy, FileAction::Copy, unavailable),
            (
                "粘贴",
                IconName::ClipboardPaste,
                FileAction::Paste,
                !self.can_paste(cx),
            ),
            (
                "重命名",
                IconName::PenLine,
                FileAction::Rename,
                unavailable || multiple,
            ),
            (
                "移到回收站",
                IconName::Trash,
                FileAction::Trash,
                unavailable,
            ),
        ]
        .into_iter()
        .fold(menu.separator(), |menu, (label, icon, action, disabled)| {
            let entry = if matches!(action, FileAction::Paste) {
                None
            } else {
                target
            };
            menu.item(file_menu_item(label, icon, action, entry, disabled, owner))
        })
    }

    pub(super) fn build_file_background_menu(
        &self,
        menu: PopupMenu,
        owner: &WeakEntity<Self>,
        cx: &gpui_kit::App,
    ) -> PopupMenu {
        if !matches!(self.browser.state, DirectoryState::Ready(_)) {
            return menu;
        }
        let directory = self.location();
        let mut menu = menu;
        if directory.is_some() {
            for (label, icon, action) in [
                ("新建文件夹", IconName::FolderPlus, FileAction::NewFolder),
                ("新建文档", IconName::FilePlus, FileAction::NewDocument),
            ] {
                menu = menu.item(file_menu_item(
                    label,
                    icon,
                    action,
                    None,
                    self.file_job_running,
                    owner,
                ));
            }
        }
        menu = menu.item(file_menu_item(
            "全选",
            IconName::ListChecks,
            FileAction::SelectAll,
            None,
            self.browser.visible.is_empty(),
            owner,
        ));
        if self.can_paste(cx) {
            menu = menu.item(file_menu_item(
                "粘贴",
                IconName::ClipboardPaste,
                FileAction::Paste,
                None,
                false,
                owner,
            ));
        }
        if let Some(directory) = directory {
            menu = menu.item(file_menu_item(
                "属性",
                IconName::Info,
                FileAction::Properties,
                None,
                false,
                owner,
            ));
            menu = menu
                .separator()
                .item(external_terminal_item(directory.to_owned(), owner));
        }
        menu
    }
}

fn external_terminal_item(path: std::path::PathBuf, owner: &WeakEntity<App>) -> PopupMenuItem {
    let app = owner.clone();
    PopupMenuItem::new("打开外部终端")
        .icon(Icon::new(IconName::SquareTerminal))
        .on_click(move |_, _, cx| {
            let _ = app.update(cx, |this, cx| {
                this.open_external_terminal(path.clone(), cx);
            });
        })
}

fn file_menu_item(
    label: &'static str,
    icon: IconName,
    action: FileAction,
    entry: Option<(u64, usize)>,
    disabled: bool,
    owner: &WeakEntity<App>,
) -> PopupMenuItem {
    let app = owner.clone();
    PopupMenuItem::new(label)
        .icon(Icon::new(icon))
        .disabled(disabled)
        .on_click(move |_, window, cx| {
            let _ = app.update(cx, |this, cx| this.file_action(action, entry, window, cx));
        })
}
