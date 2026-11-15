use std::path::PathBuf;

use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{
    AppConfig, bookmark::Bookmarks, tab::Location, view::ViewMode, workspace::Workspace,
};
use settings::tag::Tag;
use settings::tag_store::TagStore;

use super::bookmark_menu::BookmarkMenu;
use super::bookmarks::BookmarkRename;
use super::browser::Browser;
use super::preferences::{OpenPreferences, Preferences, shortcut_bindings};
use super::tabs::{BrowserTab, CloseTab, NewTab, restore_tabs};
use super::tags::TagEditor;
use super::volumes::StorageDevices;
use crate::app::window_resize;
use crate::components::ItemContextMenu;
use crate::model::{SidebarItem, default_favorites, home_directory};
use crate::style::{
    APP_ID, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, WINDOW_HEIGHT, WINDOW_RADIUS, WINDOW_WIDTH,
};

/// 文件管理器主视图，持有导航历史、搜索输入和显示设置。
pub struct App {
    focus: FocusHandle,
    pub(super) tabs: Vec<BrowserTab>,
    pub(super) active_tab: usize,
    pub(super) workspace: Workspace,
    pub(super) bookmarks: Entity<Bookmarks>,
    pub(super) bookmark_menu: Option<BookmarkMenu>,
    pub(super) bookmark_rename: Option<BookmarkRename>,
    pub(super) bookmark_properties: Option<uuid::Uuid>,
    pub(super) persist_session: bool,
    pub(super) tags: Tag,
    pub(super) tag_store: Option<TagStore>,
    pub(super) tag_error: Option<String>,
    pub(super) workspace_writable: bool,
    pub(super) workspace_error: Option<String>,
    pub(super) tag_editor: Option<TagEditor>,
    pub(super) storage: StorageDevices,
    pub(super) default_favorites: Vec<SidebarItem>,
    pub(super) search: Entity<InputState>,
    pub(super) browser: Browser,
    pub(super) file_item_menu: ItemContextMenu,
    pub(super) view_mode: ViewMode,
    pub(super) config: AppConfig,
    pub(super) preferences: Option<WindowHandle<Preferences>>,
}

impl App {
    pub fn open_window_at(
        cx: &mut gpui_kit::App,
        target: Option<(PathBuf, Entity<Bookmarks>)>,
    ) -> anyhow::Result<WindowHandle<Self>> {
        let options = WindowOptions {
            app_id: Some(APP_ID.into()),
            window_bounds: Some(WindowBounds::centered(
                size(WINDOW_WIDTH, WINDOW_HEIGHT),
                cx,
            )),
            window_min_size: Some(size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)),
            window_background: WindowBackgroundAppearance::Transparent,
            window_decorations: Some(WindowDecorations::Client),
            ..TitleBar::window_options()
        };
        cx.open_window(options, |window, cx| {
            cx.new(|cx| Self::new(window, target, cx))
        })
    }

    fn new(
        window: &mut Window,
        target: Option<(PathBuf, Entity<Bookmarks>)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let home = home_directory();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索当前页"));
        let mut config = AppConfig::load_from_file().unwrap_or_else(|error| {
            eprintln!("无法加载配置：{error}");
            AppConfig::default()
        });
        let persist_session = target.is_none();
        if persist_session {
            match shortcut_bindings(&config.shortcuts, cx) {
                Ok(bindings) => cx.bind_keys(bindings),
                Err(error) => {
                    eprintln!("无法加载快捷键：{error}");
                    config.shortcuts = Default::default();
                    cx.bind_keys([
                        KeyBinding::new("ctrl-t", NewTab, Some("FileManager")),
                        KeyBinding::new("ctrl-w", CloseTab, Some("FileManager")),
                    ]);
                }
            }
        }
        let (mut workspace, workspace_error) = match Workspace::load() {
            Ok(workspace) => (workspace, None),
            Err(error) => (
                Workspace::default(),
                Some(format!("无法加载工作区：{error}")),
            ),
        };
        let (tag_store, tags, tag_error) = match TagStore::open() {
            Ok((store, tags)) => (Some(store), tags, None),
            Err(error) => (
                None,
                Tag::default(),
                Some(format!("无法打开标签数据库：{error}")),
            ),
        };
        let (tabs, active_tab, bookmarks) = if let Some((path, bookmarks)) = target {
            (
                vec![BrowserTab::new(
                    uuid::Uuid::new_v4(),
                    Some(Location::directory(path)),
                )],
                0,
                bookmarks,
            )
        } else {
            let (tabs, active_tab) =
                restore_tabs(&workspace, tag_store.as_ref().map(|_| &tags), home.clone());
            let bookmarks = cx.new(|_| std::mem::take(&mut workspace.bookmarks));
            (tabs, active_tab, bookmarks)
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut app = Self {
            focus,
            tabs,
            active_tab,
            workspace,
            bookmarks,
            bookmark_menu: None,
            bookmark_rename: None,
            bookmark_properties: None,
            persist_session,
            tags,
            tag_store,
            tag_error,
            workspace_writable: workspace_error.is_none(),
            workspace_error,
            tag_editor: None,
            storage: StorageDevices::default(),
            default_favorites: default_favorites(home),
            search,
            browser: Browser::new(config.view.show_hidden_files),
            file_item_menu: ItemContextMenu::default(),
            view_mode: config.view.view_mode,
            config,
            preferences: None,
        };
        cx.observe(&app.search, |this, search, cx| {
            let query = search.read(cx).value().trim().to_lowercase();
            this.search_directory(query, cx);
        })
        .detach();
        cx.observe(&app.bookmarks, |_, _, cx| cx.notify()).detach();
        app.watch_volumes(cx);
        app.load_directory(cx);
        app
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let square = window.is_maximized()
            || window.is_fullscreen()
            || matches!(window.window_decorations(), Decorations::Client { tiling } if tiling.is_tiled());
        // 窗口最大化或全屏时，圆角为 0，避免与系统边框冲突
        let radius = if square { px(0.) } else { WINDOW_RADIUS };
        let border = if square { px(0.) } else { px(1.) };
        v_flex()
            .id("file-manager")
            .key_context("FileManager")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &OpenPreferences, _, cx| this.open_preferences(cx)))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.open_new_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.close_tab(this.active_tab, window, cx);
            }))
            .relative()
            .size_full()
            .rounded(radius)
            .border(border)
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.title_bar(window, radius, cx))
            .child(
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(radius, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .when(self.show_tab_bar(), |panel| {
                                panel.child(self.tab_bar(window, cx))
                            })
                            .when_some(self.tag_editor.as_ref(), |panel, editor| {
                                panel.child(self.tag_form(editor, cx))
                            })
                            .child(self.file_area(window, cx)),
                    ),
            )
            .when(
                !square
                    && window.is_resizable()
                    && matches!(window.window_decorations(), Decorations::Client { .. }),
                |frame| frame.children(window_resize::handles()),
            )
            .children(self.file_item_menu.element())
            .children(self.bookmark_menu_view(cx))
            .children(self.bookmark_rename_view(cx))
            .children(self.bookmark_properties_dialog(cx))
    }
}
