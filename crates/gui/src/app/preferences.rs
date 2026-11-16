use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, Selectable as _};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{
    AppConfig, ViewConfig,
    action::{BlankDoubleClickAction, FolderMiddleClickAction, TabMiddleClickAction},
    shortcuts::ShortcutConfig,
    view::ViewMode,
};

use super::App;
use super::io_queue::IoQueue;
use super::tabs::{CloseTab, NewTab};
use super::window_resize;
use crate::components::preferences_title_bar;
use crate::style::{APP_ID, WINDOW_RADIUS, preferences_palette};

actions!(browser, [OpenPreferences]);

pub(super) struct ActivePreferences(pub AppConfig);

impl Global for ActivePreferences {}

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

#[derive(Clone)]
struct PreferenceValues {
    shortcuts: ShortcutConfig,
    view: ViewConfig,
    middle_click_tab: TabMiddleClickAction,
    middle_click_folder: FolderMiddleClickAction,
    blank_double_click: BlankDoubleClickAction,
}

impl PreferenceValues {
    fn merge_into(self, baseline: Self, config: &mut AppConfig) {
        if self.shortcuts.new_tab != baseline.shortcuts.new_tab {
            config.shortcuts.new_tab = self.shortcuts.new_tab;
        }
        if self.shortcuts.close_tab != baseline.shortcuts.close_tab {
            config.shortcuts.close_tab = self.shortcuts.close_tab;
        }
        if self.view.view_mode != baseline.view.view_mode {
            config.view.view_mode = self.view.view_mode;
        }
        if self.view.show_hidden_files != baseline.view.show_hidden_files {
            config.view.show_hidden_files = self.view.show_hidden_files;
        }
        if self.blank_double_click != baseline.blank_double_click {
            config.actions.blank_double_click = self.blank_double_click;
        }
        if self.middle_click_folder != baseline.middle_click_folder {
            config.actions.middle_click_folder = self.middle_click_folder;
        }
        if self.middle_click_tab != baseline.middle_click_tab {
            config.actions.middle_click_tab = self.middle_click_tab;
        }
    }
}

/// 设置窗口持有自己的草稿，只有保存成功才更新文件管理器。
pub(super) struct Preferences {
    owner: WeakEntity<App>,
    baseline: PreferenceValues,
    saving: bool,
    focus: FocusHandle,
    page: Page,
    view: ViewConfig,
    middle_click_tab: TabMiddleClickAction,
    middle_click_folder: FolderMiddleClickAction,
    blank_double_click: BlankDoubleClickAction,
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
        let config = &cx.global::<ActivePreferences>().0;
        let view = config.view.clone();
        let middle_click_tab = config.actions.middle_click_tab;
        let middle_click_folder = config.actions.middle_click_folder;
        let blank_double_click = config.actions.blank_double_click;
        let shortcuts = config.shortcuts.clone();
        let baseline = PreferenceValues {
            view: view.clone(),
            middle_click_tab,
            middle_click_folder,
            blank_double_click,
            shortcuts: shortcuts.clone(),
        };
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
                    baseline,
                    saving: false,
                    focus,
                    page: Page::View,
                    view,
                    middle_click_tab,
                    middle_click_folder,
                    blank_double_click,
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
}

fn install_preferences(config: AppConfig, cx: &mut gpui_kit::App) -> Result<(), String> {
    let bindings = shortcut_bindings(&config.shortcuts, cx)?;
    let old = &cx.global::<ActivePreferences>().0.shortcuts;
    let unbind = [
        key_binding(&old.new_tab, Unbind(NewTab.name().into()), cx)?,
        key_binding(&old.close_tab, Unbind(CloseTab.name().into()), cx)?,
    ];
    cx.bind_keys(unbind.into_iter().chain(bindings));
    cx.set_global(ActivePreferences(config));
    Ok(())
}

impl Preferences {
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let draft = PreferenceValues {
            shortcuts: ShortcutConfig {
                new_tab: self.new_tab.read(cx).value().trim().to_ascii_lowercase(),
                close_tab: self.close_tab.read(cx).value().trim().to_ascii_lowercase(),
            },
            view: self.view.clone(),
            middle_click_tab: self.middle_click_tab,
            middle_click_folder: self.middle_click_folder,
            blank_double_click: self.blank_double_click,
        };
        let baseline = self.baseline.clone();
        let handle = window.window_handle().downcast::<Self>();
        let owner = self.owner.clone();
        let update_mode = draft.view.view_mode != baseline.view.view_mode;
        let update_hidden = draft.view.show_hidden_files != baseline.view.show_hidden_files;
        self.saving = true;
        self.error = None;
        IoQueue::submit(
            cx,
            move || {
                AppConfig::update_file(|config| {
                    draft.merge_into(baseline, config);
                    validate_shortcuts(&config.shortcuts).map_err(settings::Error::InvalidShortcuts)
                })
            },
            move |result, cx| {
                let result = result
                    .map_err(|error| error.to_string())
                    .and_then(|config| {
                        let view = config.view.clone();
                        install_preferences(config, cx)?;
                        let _ = owner.update(cx, |owner, cx| {
                            if update_mode {
                                owner.set_view_mode(view.view_mode, cx);
                            }
                            if update_hidden {
                                owner.set_show_hidden_files(view.show_hidden_files, cx);
                            }
                        });
                        Ok(())
                    });
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |this, window, cx| match result {
                        Ok(()) => window.remove_window(),
                        Err(error) => {
                            this.saving = false;
                            this.error = Some(error);
                            cx.notify();
                        }
                    });
                }
            },
        );
        cx.notify();
    }

    fn sidebar(&self, radius: Pixels, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let palette = preferences_palette(cx.theme().mode);
        v_flex()
            .w(PREFERENCES_SIDEBAR_WIDTH)
            .flex_shrink_0()
            .px_3()
            .py_4()
            .gap_1()
            .rounded_bl(radius)
            .bg(palette.sidebar)
            .border_r_1()
            .border_color(palette.border)
            .children(Page::ALL.into_iter().map(|page| {
                let (label, icon) = page.appearance();
                let active = self.page == page;
                h_flex()
                    .id(label)
                    .role(Role::Tab)
                    .aria_selected(active)
                    .h_9()
                    .gap_2()
                    .px_3()
                    .rounded_md()
                    .cursor_pointer()
                    .when(active, |row| {
                        row.bg(palette.selected)
                            .text_color(palette.selected_text)
                            .font_semibold()
                    })
                    .when(!active, |row| row.hover(|style| style.bg(palette.hover)))
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
                preference_segments(
                    cx,
                    [(ViewMode::List, "列表"), (ViewMode::Grid, "网格")]
                        .into_iter()
                        .map(|(mode, label)| {
                            preference_choice(label, label, self.view.view_mode == mode, cx)
                                .disabled(self.saving)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.view.view_mode = mode;
                                    cx.notify();
                                }))
                        }),
                ),
            ))
            .child(preference_row(
                "显示隐藏文件",
                Switch::new("hidden-files")
                    .accessibility_label("显示隐藏文件")
                    .color(rgb(if cx.theme().mode.is_dark() {
                        0x767676
                    } else {
                        0x454340
                    }))
                    .checked(self.view.show_hidden_files)
                    .disabled(self.saving)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        this.view.show_hidden_files = *checked;
                        cx.notify();
                    })),
            ))
    }

    fn behavior_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .gap_3()
            .child(preference_row(
                "双击文件区空白处",
                preference_segments(
                    cx,
                    [
                        (
                            "blank-double-click-up",
                            "返回上一级",
                            BlankDoubleClickAction::GoUp,
                        ),
                        (
                            "blank-double-click-refresh",
                            "刷新",
                            BlankDoubleClickAction::Refresh,
                        ),
                        (
                            "blank-double-click-none",
                            "无操作",
                            BlankDoubleClickAction::None,
                        ),
                    ]
                    .into_iter()
                    .map(|(id, label, action)| {
                        preference_choice(id, label, self.blank_double_click == action, cx)
                            .disabled(self.saving)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.blank_double_click = action;
                                cx.notify();
                            }))
                    }),
                ),
            ))
            .child(preference_row(
                "中键点击标签页",
                preference_segments(
                    cx,
                    [
                        ("middle-click-close", "关闭", TabMiddleClickAction::CloseTab),
                        ("middle-click-copy", "复制", TabMiddleClickAction::CopyTab),
                        ("middle-click-none", "无操作", TabMiddleClickAction::None),
                    ]
                    .into_iter()
                    .map(|(id, label, action)| {
                        preference_choice(id, label, self.middle_click_tab == action, cx)
                            .disabled(self.saving)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.middle_click_tab = action;
                                cx.notify();
                            }))
                    }),
                ),
            ))
            .child(preference_row(
                "中键点击文件夹",
                preference_segments(
                    cx,
                    [
                        (
                            "folder-middle-click-tab",
                            "新标签页",
                            FolderMiddleClickAction::OpenInNewTab,
                        ),
                        (
                            "folder-middle-click-window",
                            "新窗口",
                            FolderMiddleClickAction::OpenInNewWindow,
                        ),
                        (
                            "folder-middle-click-none",
                            "无操作",
                            FolderMiddleClickAction::None,
                        ),
                    ]
                    .into_iter()
                    .map(|(id, label, action)| {
                        preference_choice(id, label, self.middle_click_folder == action, cx)
                            .disabled(self.saving)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.middle_click_folder = action;
                                cx.notify();
                            }))
                    }),
                ),
            ))
    }

    fn shortcuts_page(&self, cx: &gpui_kit::App) -> impl IntoElement + use<> {
        v_flex()
            .gap_3()
            .child(preference_row(
                "新建标签页",
                div()
                    .w(px(176.))
                    .child(Input::new(&self.new_tab).disabled(self.saving)),
            ))
            .child(preference_row(
                "关闭标签页",
                div()
                    .w(px(176.))
                    .child(Input::new(&self.close_tab).disabled(self.saving)),
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
        let palette = preferences_palette(theme.mode);
        let (foreground, danger) = (theme.foreground, theme.danger);
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
            .bg(palette.background)
            .text_color(foreground)
            .when(!square, |frame| {
                frame.border_1().border_color(palette.border)
            })
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
                                    .id((ElementId::from("preferences-content"), title))
                                    .flex_1()
                                    .min_h_0()
                                    .px_6()
                                    .py_5()
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
                                    .border_color(palette.border)
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
                                                    .disabled(self.saving)
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

fn preference_segments(
    cx: &gpui_kit::App,
    choices: impl IntoIterator<Item = Button>,
) -> impl IntoElement {
    let (track, border) = if cx.theme().mode.is_dark() {
        (0x232b2e, 0x47565b)
    } else {
        (0xe9eff0, 0xcbd8da)
    };
    h_flex()
        .flex_shrink_0()
        .gap_0p5()
        .p_1()
        .rounded_lg()
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(track))
        .children(choices)
}

fn preference_choice(
    id: &'static str,
    label: &'static str,
    selected: bool,
    cx: &gpui_kit::App,
) -> Button {
    let (background, foreground, muted, border) = if cx.theme().mode.is_dark() {
        (0x465457, 0xffffff, 0xc5d0d2, 0x778b8e)
    } else {
        (0xffffff, 0x172426, 0x46585b, 0xaebfc2)
    };
    Button::new(id)
        .ghost()
        .label(label)
        .small()
        .h_7()
        .px_3()
        .border_1()
        .border_color(transparent_black())
        .text_color(rgb(if selected { foreground } else { muted }))
        .font_weight(if selected {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        })
        .when(selected, |button| {
            button
                .bg(rgb(background))
                .border_color(rgb(border))
                .shadow_sm()
        })
        .selected(selected)
        .toggled(selected)
}

fn preference_row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    h_flex()
        .min_h_10()
        .justify_between()
        .gap_3()
        .child(div().text_sm().child(label))
        .child(control)
}

pub(super) fn shortcut_bindings(
    shortcuts: &ShortcutConfig,
    cx: &gpui_kit::App,
) -> Result<[KeyBinding; 2], String> {
    validate_shortcuts(shortcuts)?;
    Ok([
        key_binding(&shortcuts.new_tab, NewTab, cx)?,
        key_binding(&shortcuts.close_tab, CloseTab, cx)?,
    ])
}

fn validate_shortcuts(shortcuts: &ShortcutConfig) -> Result<(), String> {
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
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{
        AppConfig, BlankDoubleClickAction, PreferenceValues, ShortcutConfig, ViewConfig, ViewMode,
    };

    #[test]
    fn preference_draft_does_not_overwrite_unedited_fields() {
        let baseline = PreferenceValues {
            shortcuts: ShortcutConfig::default(),
            view: ViewConfig::default(),
            middle_click_tab: AppConfig::default().actions.middle_click_tab,
            middle_click_folder: AppConfig::default().actions.middle_click_folder,
            blank_double_click: BlankDoubleClickAction::GoUp,
        };
        let mut draft = baseline.clone();
        draft.view.show_hidden_files = true;
        let mut latest = AppConfig::default();
        latest.shortcuts.new_tab = "ctrl-shift-t".to_owned();
        latest.view.view_mode = ViewMode::Grid;
        latest.actions.blank_double_click = BlankDoubleClickAction::Refresh;

        draft.merge_into(baseline, &mut latest);

        assert_eq!(latest.shortcuts.new_tab, "ctrl-shift-t");
        assert_eq!(latest.view.view_mode, ViewMode::Grid);
        assert!(latest.view.show_hidden_files);
        assert_eq!(
            latest.actions.blank_double_click,
            BlankDoubleClickAction::Refresh
        );
    }
}
