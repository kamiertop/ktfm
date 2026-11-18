use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon,
    menu::{PopupMenu, PopupMenuItem},
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, WeakEntity, Window};
use settings::command::AppCommand;
use settings::config::{bookmark::BookmarkKind, tab::Location};
use vfs::EntryKind;

use super::App;
use super::browser::DirectoryState;
use super::file_operations::{FileAction, FileMenuTarget};
use super::preferences::ActivePreferences;

use super::menu::{MENU_ICON_EXTRA_GAP, app_menu_item};

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
        pane_id: uuid::Uuid,
        snapshot_revision: u64,
        index: usize,
        menu: PopupMenu,
        owner: &WeakEntity<Self>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let Some(entity) = owner.upgrade() else {
            return menu;
        };
        let app = entity.read(cx);
        // 切换目录后虚拟列表会复用索引，过期的卡片事件不能作用到新目录。
        if app.pane().id != pane_id || app.pane().browser.snapshot_revision != snapshot_revision {
            return menu;
        }
        let Some((path, kind)) = app.entry_target(index) else {
            return menu;
        };

        entity.update(cx, |app, cx| {
            if app.panes[app.pane_target]
                .browser
                .select_for_context_menu(index)
            {
                cx.notify();
            }
        });
        let app = entity.read(cx);
        let unavailable = app.file_job.is_some() || kind == EntryKind::Missing;
        let multiple = app.pane().browser.selection_len() > 1;
        let target = FileMenuTarget {
            pane_id,
            navigation_revision: app.pane().navigation_revision,
            path: Some(path.clone()),
        };
        let paste_target = (kind == EntryKind::Directory).then_some(index);
        let mut menu = [
            ("剪切", IconName::Scissors, FileAction::Cut, unavailable),
            ("复制", IconName::Copy, FileAction::Copy, unavailable),
            (
                if paste_target.is_some() {
                    "粘贴到此文件夹"
                } else {
                    "粘贴"
                },
                IconName::ClipboardPaste,
                FileAction::Paste,
                !app.can_paste_into(paste_target, cx),
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
        .fold(menu, |menu, (label, icon, action, disabled)| {
            menu.item(file_menu_item(
                label,
                icon,
                action,
                target.clone(),
                disabled,
                owner,
            ))
        });
        menu = menu.separator();
        if kind == EntryKind::Directory {
            let location = Location::directory(path.clone());
            let revision = target.navigation_revision;
            menu = menu
                .item(app_menu_item(
                    owner,
                    "在新标签页打开",
                    IconName::FolderOpen,
                    move |app, window, cx| {
                        if app.pane().id == pane_id && app.pane().navigation_revision == revision {
                            app.new_tab(Some(location.clone()), window, cx);
                        }
                    },
                ))
                .item(external_terminal_item(path.clone(), owner))
                .separator();
        }
        let bookmark_kind = match kind {
            EntryKind::Directory => Some(BookmarkKind::Directory),
            EntryKind::File | EntryKind::Symlink => Some(BookmarkKind::File),
            _ => None,
        };
        if let Some(bookmark_kind) = bookmark_kind
            && !app.is_default_favorite(&path)
        {
            let bookmarked = app.bookmarks.read(cx).find(&path).is_some();
            let (label, icon) = if bookmarked {
                ("从侧栏书签移除", IconName::BookmarkMinus)
            } else {
                ("添加到侧栏书签", IconName::BookmarkPlus)
            };
            let target = path.clone();
            menu = menu.item(app_menu_item(owner, label, icon, move |app, _, cx| {
                app.set_bookmark(target.clone(), bookmark_kind, !bookmarked, cx);
            }));
        }

        let current_tag = match app.current_location() {
            Some(Location::Tag { id }) => Some(*id),
            _ => None,
        };
        let tags: Vec<_> = app
            .tags(cx)
            .items()
            .iter()
            .map(|tag| (tag.id, tag.name.clone(), tag.contains(&path)))
            .collect();
        let owner = owner.clone();
        menu.submenu_with_icon(
            Some(Icon::new(IconName::Tag).mr(MENU_ICON_EXTRA_GAP)),
            "标签",
            window,
            cx,
            move |menu, _, cx| {
                super::tag_menu::build_tag_menu(
                    menu,
                    owner.clone(),
                    path.clone(),
                    current_tag,
                    &tags,
                    cx,
                )
            },
        )
    }

    pub(super) fn build_file_background_menu(
        &self,
        menu: PopupMenu,
        owner: &WeakEntity<Self>,
        cx: &gpui_kit::App,
    ) -> PopupMenu {
        if !matches!(
            self.panes[self.pane_target].browser.state,
            DirectoryState::Ready(_)
        ) {
            return menu;
        }
        let target = FileMenuTarget {
            pane_id: self.panes[self.pane_target].id,
            navigation_revision: self.panes[self.pane_target].navigation_revision,
            path: None,
        };
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
                    target.clone(),
                    self.file_job.is_some(),
                    owner,
                ));
            }
        }
        if !self.panes[self.pane_target].browser.visible.is_empty() {
            menu = menu.item(file_menu_item(
                "全选",
                IconName::ListChecks,
                FileAction::SelectAll,
                target.clone(),
                false,
                owner,
            ));
        }
        if self.can_paste(cx) {
            menu = menu.item(file_menu_item(
                "粘贴",
                IconName::ClipboardPaste,
                FileAction::Paste,
                target.clone(),
                false,
                owner,
            ));
        }
        if let Some(directory) = directory {
            menu = menu.item(file_menu_item(
                "属性",
                IconName::Info,
                FileAction::Properties,
                target.clone(),
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
    app_menu_item(
        owner,
        "打开外部终端",
        IconName::SquareTerminal,
        move |app, _, cx| {
            app.open_external_terminal(path.clone(), cx);
        },
    )
}

fn file_menu_item(
    label: &'static str,
    icon: IconName,
    action: FileAction,
    target: FileMenuTarget,
    disabled: bool,
    owner: &WeakEntity<App>,
) -> PopupMenuItem {
    app_menu_item(owner, label, icon, move |app, window, cx| {
        app.file_action(action, Some(&target), window, cx);
    })
    .disabled(disabled)
}
