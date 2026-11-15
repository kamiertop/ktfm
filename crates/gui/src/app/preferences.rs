use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, Icon, StyledExt as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{
    AppConfig, ViewConfig, action::TabMiddleClickAction, shortcuts::ShortcutConfig, view::ViewMode,
};

use super::App;
use super::tabs::{CloseTab, NewTab};
use super::window_resize;
use crate::components::preferences_title_bar;
use crate::style::{APP_ID, WINDOW_RADIUS};

actions!(browser, [OpenPreferences]);

const PREFERENCES_SIDEBAR_WIDTH: Pixels = px(176.);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    View,
    Behavior,
    Shortcuts,
    About,
}

impl Page {
    const ALL: [Self; 4] = [Self::View, Self::Behavior, Self::Shortcuts, Self::About];

    fn appearance(self) -> (&'static str, IconName) {
        match self {
            Self::View => ("文件视图", IconName::LayoutGrid),
            Self::Behavior => ("操作行为", IconName::Mouse),
            Self::Shortcuts => ("快捷键", IconName::Keyboard),
            Self::About => ("关于", IconName::Info),
        }
    }
}

/// 设置窗口持有自己的草稿，只有保存成功才更新文件管理器。
pub(super) struct Preferences {
    owner: WeakEntity<App>,
    focus: FocusHandle,
    page: Page,
    view: ViewConfig,
    middle_click_tab: TabMiddleClickAction,
    new_tab: Entity<InputState>,
    close_tab: Entity<InputState>,
    error: Option<String>,
}

impl App {
    pub(super) fn open_preferences(&mut self, cx: &mut Context<Self>) {
        if self.preferences.is_some_and(|handle| {
            handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        }) {
            return;
        }

        let owner = cx.entity();
        let view = ViewConfig {
            view_mode: self.view_mode,
            show_hidden_files: self.browser.show_hidden,
        };
        let middle_click_tab = self.config.actions.middle_click_tab;
        let shortcuts = self.config.shortcuts.clone();
        let options = WindowOptions {
            app_id: Some(APP_ID.into()),
            window_bounds: Some(WindowBounds::centered(size(px(820.), px(580.)), cx)),
            window_min_size: Some(size(px(640.), px(460.))),
            titlebar: Some(TitlebarOptions {
                title: Some("ktfm — 偏好设置".into()),
                ..TitleBar::title_bar_options()
            }),
            window_background: WindowBackgroundAppearance::Transparent,
            window_decorations: Some(WindowDecorations::Client),
            ..TitleBar::window_options()
        };
        self.preferences = match cx.open_window(options, move |window, cx| {
            let handle = window.window_handle();
            cx.new(|cx| {
                cx.observe_release(&owner, move |_, _, cx| {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                })
                .detach();

                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                Preferences {
                    owner: owner.downgrade(),
                    focus,
                    page: Page::View,
                    view,
                    middle_click_tab,
                    new_tab: cx
                        .new(|cx| InputState::new(window, cx).default_value(shortcuts.new_tab)),
                    close_tab: cx
                        .new(|cx| InputState::new(window, cx).default_value(shortcuts.close_tab)),
                    error: None,
                }
            })
        }) {
            Ok(handle) => Some(handle),
            Err(error) => {
                self.workspace_error = Some(format!("无法打开偏好设置：{error}"));
                cx.notify();
                None
            }
        };
    }

    fn apply_preferences(
        &mut self,
        shortcuts: ShortcutConfig,
        view: ViewConfig,
        middle_click_tab: TabMiddleClickAction,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let bindings = shortcut_bindings(&shortcuts, cx)?;
        let old = &self.config.shortcuts;
        let unbind = [
            key_binding(&old.new_tab, Unbind(NewTab.name().into()), cx)?,
            key_binding(&old.close_tab, Unbind(CloseTab.name().into()), cx)?,
        ];
        // 只覆盖本页管理的字段，保留配置文件中其余选项。
        let mut config = AppConfig::load_from_file().map_err(|error| error.to_string())?;
        config.shortcuts = shortcuts;
        config.view = view;
        config.actions.middle_click_tab = middle_click_tab;
        config.save_to_file().map_err(|error| error.to_string())?;

        cx.bind_keys(unbind.into_iter().chain(bindings));
        self.set_view_mode(config.view.view_mode, cx);
        self.set_show_hidden_files(config.view.show_hidden_files, cx);
        self.config = config;
        cx.notify();
        Ok(())
    }
}

impl Preferences {
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shortcuts = ShortcutConfig {
            new_tab: self.new_tab.read(cx).value().trim().to_ascii_lowercase(),
            close_tab: self.close_tab.read(cx).value().trim().to_ascii_lowercase(),
        };
        let result = self.owner.update(cx, |owner, cx| {
            owner.apply_preferences(shortcuts, self.view.clone(), self.middle_click_tab, cx)
        });
        match result {
            Ok(Ok(())) => window.remove_window(),
            Ok(Err(error)) => {
                self.error = Some(error);
                cx.notify();
            }
            Err(_) => window.remove_window(),
        }
    }

    fn sidebar(&self, radius: Pixels, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (background, selected, border, hover) = (
            theme.sidebar,
            theme.sidebar_accent,
            theme.sidebar_border,
            theme.list_hover,
        );
        v_flex()
            .w(PREFERENCES_SIDEBAR_WIDTH)
            .flex_shrink_0()
            .px_3()
            .py_4()
            .gap_1()
            .rounded_bl(radius)
            .bg(background)
            .border_r_1()
            .border_color(border)
            .children(Page::ALL.into_iter().map(|page| {
                let (label, icon) = page.appearance();
                h_flex()
                    .id(label)
                    .h_9()
                    .gap_2()
                    .px_3()
                    .rounded_md()
                    .cursor_pointer()
                    .when(self.page == page, |row| row.bg(selected).font_semibold())
                    .when(self.page != page, |row| row.hover(|style| style.bg(hover)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.page = page;
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }))
                    .child(Icon::new(icon).size(px(17.)))
                    .child(div().text_sm().child(label))
            }))
    }

    fn view_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .gap_3()
            .child(preference_row(
                "显示方式",
                h_flex().gap_1().children(
                    [(ViewMode::List, "列表"), (ViewMode::Grid, "网格")]
                        .into_iter()
                        .map(|(mode, label)| {
                            Button::new(label)
                                .ghost()
                                .label(label)
                                .toggled(self.view.view_mode == mode)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.view.view_mode = mode;
                                    cx.notify();
                                }))
                        }),
                ),
            ))
            .child(preference_row(
                "显示隐藏文件",
                Button::new("hidden-files")
                    .ghost()
                    .label(if self.view.show_hidden_files {
                        "开启"
                    } else {
                        "关闭"
                    })
                    .toggled(self.view.show_hidden_files)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.view.show_hidden_files = !this.view.show_hidden_files;
                        cx.notify();
                    })),
            ))
    }

    fn behavior_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        preference_row(
            "中键点击标签页",
            h_flex().gap_1().children(
                [
                    (TabMiddleClickAction::CloseTab, "关闭"),
                    (TabMiddleClickAction::CopyTab, "复制"),
                    (TabMiddleClickAction::None, "无操作"),
                ]
                .into_iter()
                .map(|(action, label)| {
                    Button::new(label)
                        .ghost()
                        .label(label)
                        .toggled(self.middle_click_tab == action)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.middle_click_tab = action;
                            cx.notify();
                        }))
                }),
            ),
        )
    }

    fn shortcuts_page(&self, cx: &gpui_kit::App) -> impl IntoElement + use<> {
        v_flex()
            .gap_3()
            .child(div().font_semibold().child("标签页"))
            .child(preference_row(
                "新建标签页",
                div().w(px(176.)).child(Input::new(&self.new_tab)),
            ))
            .child(preference_row(
                "关闭标签页",
                div().w(px(176.)).child(Input::new(&self.close_tab)),
            ))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("使用 Ctrl、Alt 或 Super 组合键，例如 ctrl-shift-t。"),
            )
    }

    fn about_page(&self, cx: &gpui_kit::App) -> impl IntoElement + use<> {
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::Folder).size(px(24.)))
                    .child(div().text_xl().font_semibold().child("ktfm")),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("版本 {}", env!("CARGO_PKG_VERSION"))),
            )
            .child(
                Button::new("project-source")
                    .ghost()
                    .label("查看项目源码")
                    .on_click(|_, _, cx| cx.open_url("https://github.com/kamiertop/ktfm")),
            )
    }
}

impl Render for Preferences {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let square = window.is_maximized()
            || window.is_fullscreen()
            || matches!(window.window_decorations(), Decorations::Client { tiling } if tiling.is_tiled());
        let radius = if square { px(0.) } else { WINDOW_RADIUS };
        let theme = cx.theme();
        let (background, foreground, border, danger) = (
            theme.background,
            theme.foreground,
            theme.border,
            theme.danger,
        );
        let (title, _) = self.page.appearance();
        let content = match self.page {
            Page::View => self.view_page(cx).into_any_element(),
            Page::Behavior => self.behavior_page(cx).into_any_element(),
            Page::Shortcuts => self.shortcuts_page(cx).into_any_element(),
            Page::About => self.about_page(cx).into_any_element(),
        };

        v_flex()
            .id("preferences-window")
            .relative()
            .size_full()
            .rounded(radius)
            .bg(background)
            .text_color(foreground)
            .when(!square, |frame| frame.border_1().border_color(border))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .child(preferences_title_bar(
                radius,
                PREFERENCES_SIDEBAR_WIDTH,
                window,
                cx,
            ))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.sidebar(radius, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(
                                v_flex()
                                    .id(title)
                                    .flex_1()
                                    .min_h_0()
                                    .gap_5()
                                    .p_6()
                                    .child(div().text_xl().font_semibold().child(title))
                                    .child(content)
                                    .overflow_y_scrollbar(),
                            )
                            .child(
                                v_flex()
                                    .flex_shrink_0()
                                    .px_6()
                                    .py_3()
                                    .gap_2()
                                    .border_t_1()
                                    .border_color(border)
                                    .when_some(self.error.clone(), |footer, error| {
                                        footer
                                            .child(div().text_sm().text_color(danger).child(error))
                                    })
                                    .child(
                                        h_flex()
                                            .justify_end()
                                            .gap_2()
                                            .child(
                                                Button::new("cancel-preferences")
                                                    .ghost()
                                                    .label("取消")
                                                    .on_click(|_, window, _| {
                                                        window.remove_window()
                                                    }),
                                            )
                                            .child(
                                                Button::new("save-preferences")
                                                    .primary()
                                                    .label("保存")
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| this.save(window, cx),
                                                    )),
                                            ),
                                    ),
                            ),
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

fn preference_row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    h_flex()
        .justify_between()
        .gap_3()
        .child(div().text_sm().child(label))
        .child(control)
}

pub(super) fn shortcut_bindings(
    shortcuts: &ShortcutConfig,
    cx: &gpui_kit::App,
) -> Result<[KeyBinding; 2], String> {
    let parse = |value: &str, label: &str| {
        let key =
            Keystroke::parse(value).map_err(|error| format!("{label}的快捷键无效：{error}"))?;
        if value.split_whitespace().count() != 1
            || key.key.is_empty()
            || !(key.modifiers.control || key.modifiers.alt || key.modifiers.platform)
        {
            return Err(format!("{label}需要一个包含 Ctrl、Alt 或 Super 的组合键"));
        }
        Ok(key)
    };
    let new = parse(&shortcuts.new_tab, "新建标签页")?;
    let close = parse(&shortcuts.close_tab, "关闭标签页")?;
    let preferences = parse("ctrl-p", "偏好设置")?;
    if new == preferences || close == preferences {
        return Err("Ctrl+P 已用于打开偏好设置".into());
    }
    if new == close {
        return Err("两个操作不能使用相同的快捷键".into());
    }
    Ok([
        key_binding(&shortcuts.new_tab, NewTab, cx)?,
        key_binding(&shortcuts.close_tab, CloseTab, cx)?,
    ])
}

fn key_binding(value: &str, action: impl Action, cx: &gpui_kit::App) -> Result<KeyBinding, String> {
    let context =
        KeyBindingContextPredicate::parse("FileManager").map_err(|error| error.to_string())?;
    KeyBinding::load(
        value,
        Box::new(action),
        Some(context.into()),
        false,
        None,
        cx.keyboard_mapper().as_ref(),
    )
    .map_err(|error| error.to_string())
}
