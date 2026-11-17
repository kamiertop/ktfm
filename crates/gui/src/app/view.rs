use std::path::PathBuf;

use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{
    AppConfig, bookmark::Bookmarks, tab::Location, view::ViewMode, workspace::Workspace,
};

use super::bookmarks::{BookmarkProperties, BookmarkRename};
use super::browser::Browser;
use super::embed_terminal::TerminalResize;
use super::file_operations::{
    DirectoryProperties, FileAction, FileClipboard, FileJobs, FileNameEditor,
};
use super::io_queue::IoQueue;
use super::keymap;
use super::keymap::{CloseTab, NewTab, OpenPreferences, OpenTerminal};
use super::preferences::{ActivePreferences, Preferences};
use super::tabs::{BrowserTab, restore_tabs};
use super::tags::{TagEditor, TagState};
use super::volumes::{SharedStorage, StorageDevices};
use super::workspace::WorkspaceWriter;
use crate::app::window_resize;
use crate::model::{SidebarItem, default_favorites, home_directory};
use crate::style::{
    APP_ID, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, WINDOW_HEIGHT, WINDOW_RADIUS, WINDOW_WIDTH,
};
use settings::config::general::StartupBehavior;

/// 文件管理器主视图，持有导航历史、搜索输入和显示设置。
pub struct App {
    pub(super) focus: FocusHandle,
    pub(super) file_job: Option<daemon::job::JobKind>,
    pub(super) file_name_editor: Option<FileNameEditor>,
    pub(super) directory_properties: Option<DirectoryProperties>,
    pub(super) tabs: Vec<BrowserTab>,
    pub(super) active_tab: usize,
    pub(super) navigation_revision: u64,
    pub(super) bookmarks: Entity<Bookmarks>,
    pub(super) bookmark_rename: Option<BookmarkRename>,
    pub(super) bookmark_properties: Option<BookmarkProperties>,
    pub(super) persist_session: bool,
    pub(super) tag_state: Entity<TagState>,
    pub(super) tag_error: Option<String>,
    pub(super) workspace_writable: bool,
    pub(super) workspace_error: Option<String>,
    pub(super) tag_editor: Option<TagEditor>,
    pub(super) storage: Entity<StorageDevices>,
    pub(super) default_favorites: Vec<SidebarItem>,
    pub(super) search: Entity<InputState>,
    pub(super) browser: Browser,
    pub(super) view_mode: ViewMode,
    pub(super) preferences: Option<WindowHandle<Preferences>>,
}

struct SharedTags(Entity<TagState>);

impl Global for SharedTags {}

impl App {
    pub fn open_main_window(cx: &mut gpui_kit::App) {
        cx.set_global(FileClipboard::default());
        cx.set_global(FileJobs::default());
        let writer = WorkspaceWriter::default();
        cx.set_global(writer.clone());
        match IoQueue::new() {
            Ok(io) => cx.set_global(io),
            Err(error) => {
                eprintln!("无法启动存储线程：{error}");
                cx.quit();
                return;
            }
        }
        cx.on_app_quit(move |cx| {
            cx.global_mut::<IoQueue>().finish();
            writer.flush();
            std::future::ready(())
        })
        .detach();

        SharedStorage::install(cx);
        let tags = cx.new(|_| TagState::loading());
        cx.set_global(SharedTags(tags.clone()));
        if let Err(error) = Self::open_window_at(cx, None) {
            eprintln!("无法打开窗口：{error:#}");
            cx.quit();
            return;
        }
        TagState::load_in_background(&tags, cx);
        cx.activate(true);
    }

    pub(super) fn open_window_at(
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
        let persist_session = target.is_none();
        if persist_session {
            let mut config = AppConfig::load_from_file().unwrap_or_else(|error| {
                eprintln!("无法加载配置：{error}");
                AppConfig::default()
            });
            if let Err(error) = keymap::bind_all(&config.shortcuts, cx) {
                // 键位无效时退回默认配置，保证窗口按键始终可用。
                eprintln!("无法加载快捷键：{error}");
                config.shortcuts = Default::default();
                let _ = keymap::bind_all(&config.shortcuts, cx);
            }
            cx.set_global(ActivePreferences(config));
        }
        let (mut workspace, workspace_error) =
            match persist_session.then(Workspace::load).transpose() {
                Ok(workspace) => (workspace.unwrap_or_default(), None),
                Err(error) => (
                    Workspace::default(),
                    Some(format!("无法加载工作区：{error}")),
                ),
            };
        let tag_state = cx.global::<SharedTags>().0.clone();
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
                match &cx.global::<ActivePreferences>().0.general.startup_behavior {
                    StartupBehavior::RestoreLastDir => {
                        restore_tabs(&workspace, tag_state.read(cx).loaded_tags(), home.clone())
                    }
                    behavior => {
                        let path = match behavior {
                            StartupBehavior::OpenSpecifiedDir(path) if path.is_absolute() => {
                                Some(path.clone())
                            }
                            StartupBehavior::OpenSpecifiedDir(path) => {
                                home.as_ref().map(|home| home.join(path))
                            }
                            _ => home.clone(),
                        };
                        (
                            vec![BrowserTab::new(
                                uuid::Uuid::new_v4(),
                                path.map(Location::directory),
                            )],
                            0,
                        )
                    }
                };
            let bookmarks = cx.new(|_| std::mem::take(&mut workspace.bookmarks));
            (tabs, active_tab, bookmarks)
        };

        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let view = &cx.global::<ActivePreferences>().0.view;
        let mut app = Self {
            focus,
            file_job: None,
            file_name_editor: None,
            directory_properties: None,
            tabs,
            active_tab,
            navigation_revision: 0,
            bookmarks,
            bookmark_rename: None,
            bookmark_properties: None,
            persist_session,
            tag_state,
            tag_error: None,
            workspace_writable: workspace_error.is_none(),
            workspace_error,
            tag_editor: None,
            storage: cx.global::<SharedStorage>().0.clone(),
            default_favorites: default_favorites(home),
            search,
            browser: Browser::new(view.show_hidden_files),
            view_mode: view.view_mode,
            preferences: None,
        };
        cx.observe(&app.search, |this, search, cx| {
            let query = search.read(cx).value().trim().to_lowercase();
            this.search_directory(query, cx);
        })
        .detach();
        cx.observe_global::<ActivePreferences>(|this, cx| {
            if !cx.global::<ActivePreferences>().0.view.remember_tab_state {
                this.clear_saved_tab_states();
            }
            cx.notify();
        })
        .detach();
        cx.observe_global::<FileClipboard>(|_, cx| cx.notify())
            .detach();
        cx.observe(&app.bookmarks, |_, _, cx| cx.notify()).detach();
        if persist_session {
            cx.global::<WorkspaceWriter>()
                .clone()
                .watch_bookmarks(&app.bookmarks, cx);
        }
        cx.subscribe_in(&app.tag_state, window, |this, _, event, window, cx| {
            this.sync_tag_change(event, window, cx);
        })
        .detach();
        cx.observe(&app.storage, |_, _, cx| cx.notify()).detach();
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
            .on_action(cx.listener(|this, action: &FileAction, window, cx| {
                this.file_shortcut(*action, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &OpenTerminal, window, cx| this.toggle_terminal(window, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenPreferences, _, cx| this.open_preferences(cx)))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                if this.file_name_editor.is_none()
                    && this.directory_properties.is_none()
                    && this.tag_editor.is_none()
                {
                    this.open_new_tab(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if this.file_name_editor.is_none()
                    && this.directory_properties.is_none()
                    && this.tag_editor.is_none()
                {
                    this.close_tab(this.active_tab, window, cx);
                }
            }))
            .on_drag_move::<TerminalResize>(cx.listener(Self::resize_embedded_terminal))
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
                            .child(self.file_area(window, cx))
                            .children(self.embedded_terminal_panel(window)),
                    ),
            )
            .when(
                !square
                    && window.is_resizable()
                    && matches!(window.window_decorations(), Decorations::Client { .. }),
                |frame| frame.children(window_resize::handles()),
            )
            .children(self.bookmark_rename_view(cx))
            .children(self.bookmark_properties_dialog(cx))
            .children(self.file_name_dialog(cx))
            .children(self.directory_properties_dialog(cx))
            .children(self.tag_editor_dialog(cx))
    }
}
