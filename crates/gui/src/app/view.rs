use std::path::PathBuf;

use super::panes::{BrowserPane, PaneLayout};
use gpui_kit::component::{ActiveTheme, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{
    AppConfig,
    bookmark::Bookmarks,
    tab::{Location, SplitLayout},
    workspace::Workspace,
};

use super::bookmarks::{BookmarkProperties, BookmarkRename};
use super::file_operations::{FileAction, FileClipboard, FileJobs, FileNameEditor, FileProperties};
use super::io_queue::IoQueue;
use super::keymap;
use super::keymap::{
    ClosePane, CloseTab, NewTab, OpenPreferences, OpenTerminal, PaneDown, PaneLeft, PaneRight,
    PaneUp, SplitDown, SplitLeft, SplitPane, SplitUp,
};
use super::preferences::{ActivePreferences, Preferences};
use super::tabs::{BrowserTab, restore_tabs};
use super::tags::{TagEditor, TagState};
use super::thumbnails::Thumbnails;
use super::volumes::{SharedStorage, StorageDevices, VolumeUnlock, VolumeUnmounted};
use super::window_geometry::{self, GeometrySaver};
use super::workspace::WorkspaceWriter;
use crate::app::window_resize;
use crate::model::{SidebarItem, default_favorites, home_directory};
use crate::style::{APP_ID, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, WINDOW_RADIUS};
use settings::config::general::StartupBehavior;

/// 文件管理器主视图，持有导航历史、搜索输入和显示设置。
pub struct App {
    pub(super) panes: Vec<BrowserPane>,
    pub(super) thumbnails: Thumbnails,
    pub(super) pane_target: usize,
    pub(super) active_pane: usize,
    pub(super) pane_layout: PaneLayout,
    pub(super) focus: FocusHandle,
    pub(super) file_job: Option<daemon::job::JobKind>,
    pub(super) file_name_editor: Option<FileNameEditor>,
    pub(super) file_properties: Option<FileProperties>,
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
    pub(super) volume_unlock: Option<VolumeUnlock>,
    pub(super) default_favorites: Vec<SidebarItem>,
    pub(super) preferences: Option<WindowHandle<Preferences>>,
}

struct SharedTags(Entity<TagState>);

impl Global for SharedTags {}

impl App {
    pub fn open_main_window(cx: &mut gpui_kit::App) {
        cx.set_global(FileClipboard::default());
        keymap::bind_fixed_keys(cx);
        cx.set_global(FileJobs::default());
        let writer = WorkspaceWriter::default();
        cx.set_global(writer.clone());
        match IoQueue::new() {
            Ok(io) => cx.set_global(io),
            Err(error) => {
                tracing::error!(%error, "无法启动存储线程");
                cx.quit();
                return;
            }
        }
        cx.set_global(GeometrySaver::default());
        cx.on_app_quit(move |cx| {
            GeometrySaver::flush(cx);
            cx.global_mut::<IoQueue>().finish();
            writer.flush();
            std::future::ready(())
        })
        .detach();

        SharedStorage::install(cx);
        let tags = cx.new(|_| TagState::loading());
        cx.set_global(SharedTags(tags.clone()));
        if let Err(error) = Self::open_window_at(cx, None) {
            tracing::error!(error = %format_args!("{error:#}"), "无法打开窗口");
            cx.quit();
            return;
        }
        TagState::load_in_background(&tags, cx);
        cx.activate(true);
    }

    pub(super) fn open_window_at(
        cx: &mut gpui_kit::App,
        target: Option<(PathBuf, Entity<Bookmarks>)>,
    ) -> Result<WindowHandle<Self>> {
        let workspace = Workspace::load();
        let saved_size = workspace
            .as_ref()
            .ok()
            .and_then(|workspace| workspace.window_size);
        let options = WindowOptions {
            app_id: Some(APP_ID.into()),
            // 恢复上次记住的窗口大小，没有记录时退回默认大小
            window_bounds: Some(window_geometry::startup_bounds(saved_size, cx)),
            window_min_size: Some(size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)),
            window_background: WindowBackgroundAppearance::Transparent,
            window_decorations: Some(WindowDecorations::Client),
            ..TitleBar::window_options()
        };
        cx.open_window(options, |window, cx| {
            cx.new(|cx| Self::new(window, target, workspace, cx))
        })
    }

    fn new(
        window: &mut Window,
        target: Option<(PathBuf, Entity<Bookmarks>)>,
        workspace: settings::Result<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let home = home_directory();
        let persist_session = target.is_none();
        if persist_session {
            let mut config = AppConfig::load_from_file().unwrap_or_else(|error| {
                tracing::warn!(%error, "无法加载配置");
                AppConfig::default()
            });
            if let Err(error) = keymap::bind_all(&config.shortcuts, cx) {
                // 键位无效时退回默认配置，保证窗口按键始终可用。
                tracing::warn!(%error, "无法加载快捷键");
                config.shortcuts = Default::default();
                let _ = keymap::bind_all(&config.shortcuts, cx);
            }
            cx.set_global(ActivePreferences(config));
        }
        let (mut workspace, workspace_error) = match workspace {
            Ok(workspace) => (workspace, None),
            Err(error) if !persist_session => {
                tracing::warn!(%error, "无法读取窗口大小");
                (Workspace::default(), None)
            }
            Err(error) => (
                Workspace::default(),
                Some(format!("无法加载工作区：{error}")),
            ),
        };
        if persist_session {
            cx.set_global(super::file_table::FileColumnPreferences(
                workspace.file_list_columns.clone().normalize(),
            ));
        }
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
        let mut pane = BrowserPane::new(tabs, active_tab, window, cx);
        pane.restore_view(workspace.session.view_mode, workspace.session.zoom_step);
        let mut app = Self {
            focus,
            file_job: None,
            file_name_editor: None,
            file_properties: None,
            panes: vec![pane],
            thumbnails: Thumbnails::new(),
            pane_target: 0,
            active_pane: 0,
            pane_layout: PaneLayout::default(),
            bookmarks,
            bookmark_rename: None,
            bookmark_properties: None,
            persist_session,
            tag_state,
            tag_error: None,
            workspace_writable: workspace_error.is_none(),
            workspace_error,
            tag_editor: None,
            storage: cx.global::<SharedStorage>().state.clone(),
            volume_unlock: None,
            default_favorites: default_favorites(home),
            preferences: None,
        };
        cx.observe_global::<super::file_table::FileColumnPreferences>(|_, cx| cx.notify())
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
            this.sync_tags_in_panes(event, window, cx);
        })
        .detach();
        cx.observe(&app.storage, |_, _, cx| cx.notify()).detach();
        cx.subscribe_in(
            &app.storage,
            window,
            |app, _, event: &VolumeUnmounted, window, cx| {
                app.leave_unmounted_volume(&event.0, window, cx);
            },
        )
        .detach();
        GeometrySaver::observe(window, cx);
        app.load_directory(cx);
        if persist_session
            && matches!(
                cx.global::<ActivePreferences>().0.general.startup_behavior,
                StartupBehavior::RestoreLastDir
            )
        {
            app.restore_split(
                workspace
                    .session
                    .grid
                    .or_else(|| workspace.session.split.map(Into::into)),
                window,
                cx,
            );
        }
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
        // 窗口最大化或全屏时，边框为 0
        let border = if square { px(0.) } else { px(1.) };
        let pending_cut = cx.global::<FileClipboard>().can_cancel_cut();
        v_flex()
            .id("file-manager")
            .font_family(cx.theme().font_family.clone())
            .key_context(if pending_cut {
                "FileManager PendingFileCut"
            } else {
                "FileManager"
            })
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &keymap::CancelCut, window, cx| {
                if this.focus.is_focused(window) && cx.global::<FileClipboard>().can_cancel_cut() {
                    cx.stop_propagation();
                    cx.global_mut::<FileClipboard>().cancel_cut();
                }
            }))
            .on_action(cx.listener(|this, action: &FileAction, window, cx| {
                this.file_shortcut(*action, window, cx);
            }))
            .on_action(cx.listener(|this, _: &keymap::ZoomIn, window, cx| {
                this.file_zoom_shortcut(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &keymap::ZoomOut, window, cx| {
                this.file_zoom_shortcut(-1, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &OpenTerminal, window, cx| this.toggle_terminal(window, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenPreferences, _, cx| this.open_preferences(cx)))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                if this.file_name_editor.is_none()
                    && this.file_properties.is_none()
                    && this.tag_editor.is_none()
                    && this.volume_unlock.is_none()
                {
                    this.open_new_tab(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if this.file_name_editor.is_none()
                    && this.file_properties.is_none()
                    && this.tag_editor.is_none()
                    && this.volume_unlock.is_none()
                {
                    this.close_tab(this.panes[this.pane_target].active_tab, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SplitPane, window, cx| {
                this.open_split(SplitLayout::SideBySide, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| {
                this.open_split(SplitLayout::Stacked, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitLeft, window, cx| {
                this.open_split(SplitLayout::SideBySide, false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitUp, window, cx| {
                this.open_split(SplitLayout::Stacked, false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PaneLeft, window, cx| {
                this.focus_split_pane(SplitLayout::SideBySide, false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PaneRight, window, cx| {
                this.focus_split_pane(SplitLayout::SideBySide, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PaneUp, window, cx| {
                this.focus_split_pane(SplitLayout::Stacked, false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PaneDown, window, cx| {
                this.focus_split_pane(SplitLayout::Stacked, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ClosePane, window, cx| {
                this.close_split(window, cx);
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
                    .child(self.browser_panes(border, window, cx)),
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
            .children(self.file_properties_dialog(cx))
            .children(self.tag_editor_dialog(cx))
            .children(self.volume_unlock_dialog(cx))
    }
}
