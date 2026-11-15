use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{AppConfig, view::ViewMode, workspace::Workspace};
use settings::tag::Tag;
use settings::tag_store::TagStore;

use super::browser::Browser;
use super::tabs::{BrowserTab, NewTab, restore_tabs};
use super::tags::TagEditor;
use super::volumes::StorageDevices;
use crate::app::window_resize;
use crate::model::{SidebarItem, default_favorites, home_directory};
use crate::style::WINDOW_RADIUS;

/// 文件管理器主视图，持有导航历史、搜索输入和显示设置。
pub struct App {
    focus: FocusHandle,
    pub(super) tabs: Vec<BrowserTab>,
    pub(super) active_tab: usize,
    pub(super) workspace: Workspace,
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
    pub(super) view_mode: ViewMode,
    pub(super) show_settings: bool,
}

impl App {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = home_directory();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索当前页"));
        let config = AppConfig::load_from_file().unwrap_or_else(|error| {
            eprintln!("无法加载配置：{error}");
            AppConfig::default()
        });
        let (workspace, workspace_error) = match Workspace::load() {
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
        let (tabs, active_tab) =
            restore_tabs(&workspace, tag_store.as_ref().map(|_| &tags), home.clone());
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut app = Self {
            focus,
            tabs,
            active_tab,
            workspace,
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
            view_mode: config.view.view_mode,
            show_settings: false,
        };
        cx.observe(&app.search, |this, search, cx| {
            let query = search.read(cx).value().trim().to_lowercase();
            this.search_directory(query, cx);
        })
        .detach();
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
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.open_new_tab(window, cx)))
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
    }
}
