use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, Selectable as _};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
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
    terminal::{
        DEFAULT_FONT_SIZE, FONT_SIZE_RANGE, TerminalConfig, TerminalCursorShape, TerminalMode,
    },
    view::ViewMode,
};

use super::App;
use super::embed_terminal::parse_background;
use super::external_terminal::OpenTerminal;
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
    Terminal,
    About,
}

impl Page {
    const ALL: [Self; 5] = [
        Self::View,
        Self::Behavior,
        Self::Terminal,
        Self::Shortcuts,
        Self::About,
    ];

    fn appearance(self) -> (&'static str, IconName) {
        match self {
            Self::View => ("文件视图", IconName::LayoutGrid),
            Self::Behavior => ("操作行为", IconName::Mouse),
            Self::Terminal => ("终端", IconName::Terminal),
            Self::Shortcuts => ("快捷键", IconName::Keyboard),
            Self::About => ("关于", IconName::Info),
        }
    }
}

#[derive(Clone, PartialEq)]
struct PreferenceValues {
    shortcuts: ShortcutConfig,
    terminal: TerminalConfig,
    view: ViewConfig,
    middle_click_tab: TabMiddleClickAction,
    middle_click_folder: FolderMiddleClickAction,
    blank_double_click: BlankDoubleClickAction,
}

impl From<&AppConfig> for PreferenceValues {
    fn from(config: &AppConfig) -> Self {
        Self {
            shortcuts: config.shortcuts.clone(),
            terminal: config.terminal.clone(),
            view: config.view.clone(),
            middle_click_tab: config.actions.middle_click_tab,
            middle_click_folder: config.actions.middle_click_folder,
            blank_double_click: config.actions.blank_double_click,
        }
    }
}

impl PreferenceValues {
    fn merge_into(self, baseline: Self, config: &mut AppConfig) {
        if self.shortcuts.new_tab != baseline.shortcuts.new_tab {
            config.shortcuts.new_tab = self.shortcuts.new_tab;
        }
        if self.shortcuts.close_tab != baseline.shortcuts.close_tab {
            config.shortcuts.close_tab = self.shortcuts.close_tab;
        }
        if self.shortcuts.open_terminal != baseline.shortcuts.open_terminal {
            config.shortcuts.open_terminal = self.shortcuts.open_terminal;
        }
        if self.terminal.mode != baseline.terminal.mode {
            config.terminal.mode = self.terminal.mode;
        }
        if self.terminal.external_program != baseline.terminal.external_program {
            config.terminal.external_program = self.terminal.external_program;
        }
        if self.terminal.copy_shortcut != baseline.terminal.copy_shortcut {
            config.terminal.copy_shortcut = self.terminal.copy_shortcut;
        }
        if self.terminal.paste_shortcut != baseline.terminal.paste_shortcut {
            config.terminal.paste_shortcut = self.terminal.paste_shortcut;
        }
        if self.terminal.background != baseline.terminal.background {
            config.terminal.background = self.terminal.background;
        }
        if self.terminal.cursor_shape != baseline.terminal.cursor_shape {
            config.terminal.cursor_shape = self.terminal.cursor_shape;
        }
        if self.terminal.font_size != baseline.terminal.font_size {
            config.terminal.font_size = self.terminal.font_size;
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShortcutField {
    NewTab,
    CloseTab,
    OpenTerminal,
    Copy,
    Paste,
}

impl ShortcutField {
    fn label(self) -> &'static str {
        match self {
            Self::NewTab => "新建标签页",
            Self::CloseTab => "关闭标签页",
            Self::OpenTerminal => "打开终端",
            Self::Copy => "内置终端复制",
            Self::Paste => "内置终端粘贴",
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
    draft: PreferenceValues,
    recording: Option<ShortcutField>,
    shortcut_focus: FocusHandle,
    choosing_program: bool,
    terminal_background: Entity<InputState>,
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
        let baseline = PreferenceValues::from(&cx.global::<ActivePreferences>().0);
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
                let shortcut_focus = cx.focus_handle();
                cx.on_focus_out(
                    &shortcut_focus,
                    window,
                    |this: &mut Preferences, _, _, cx| {
                        if this.recording.take().is_some() {
                            cx.notify();
                        }
                    },
                )
                .detach();
                let terminal_background = cx.new(|cx| {
                    InputState::new(window, cx).default_value(baseline.terminal.background.clone())
                });
                cx.subscribe_in(
                    &terminal_background,
                    window,
                    |this: &mut Preferences, input, event, window, cx| {
                        if !matches!(event, InputEvent::Change) || this.embedded_disabled() {
                            return;
                        }
                        let value = input.read(cx).value().trim().to_owned();
                        if parse_background(&value).is_none() {
                            this.error =
                                Some("终端背景色需要使用 #RRGGBB 格式，例如 #161A22".into());
                            cx.notify();
                            return;
                        }
                        this.change(window, cx, |draft| draft.terminal.background = value);
                    },
                )
                .detach();

                Preferences {
                    owner: owner.downgrade(),
                    draft: baseline.clone(),
                    baseline,
                    saving: false,
                    focus,
                    page: Page::View,
                    shortcut_focus,
                    recording: None,
                    choosing_program: false,
                    terminal_background,
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
    let old = &cx.global::<ActivePreferences>().0.shortcuts;
    if config.shortcuts != *old {
        let bindings = shortcut_bindings(&config.shortcuts, cx)?;
        let unbind = [
            key_binding(&old.new_tab, Unbind(NewTab.name().into()), false, cx)?,
            key_binding(&old.close_tab, Unbind(CloseTab.name().into()), false, cx)?,
            key_binding(
                &old.open_terminal,
                Unbind(OpenTerminal.name().into()),
                true,
                cx,
            )?,
        ];
        cx.bind_keys(unbind.into_iter().chain(bindings));
    }
    cx.set_global(ActivePreferences(config));
    Ok(())
}

impl Preferences {
    fn busy(&self) -> bool {
        self.saving || self.choosing_program
    }

    fn embedded_disabled(&self) -> bool {
        self.busy() || self.draft.terminal.mode != TerminalMode::Embedded
    }

    fn change(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut PreferenceValues),
    ) {
        if !self.busy() {
            update(&mut self.draft);
            self.save(window, cx);
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.error = None;
        let draft = self.draft.clone();
        let baseline = self.baseline.clone();
        if draft == baseline {
            cx.notify();
            return;
        }
        let handle = window.window_handle().downcast::<Self>();
        let owner = self.owner.clone();
        let update_mode = draft.view.view_mode != baseline.view.view_mode;
        let update_hidden = draft.view.show_hidden_files != baseline.view.show_hidden_files;
        self.saving = true;
        IoQueue::submit(
            cx,
            move || {
                let program = &draft.terminal.external_program;
                if *program != baseline.terminal.external_program
                    && !program.is_empty()
                    && sys::opener::split_command(program)
                        .and_then(|(name, _)| sys::opener::resolve_executable(&name))
                        .is_none()
                {
                    return Err(settings::Error::InvalidExternalTerminal(program.clone()));
                }
                let check_shortcuts = draft.shortcuts != baseline.shortcuts
                    || draft.terminal.copy_shortcut != baseline.terminal.copy_shortcut
                    || draft.terminal.paste_shortcut != baseline.terminal.paste_shortcut;
                AppConfig::update_file(|config| {
                    draft.merge_into(baseline, config);
                    if check_shortcuts {
                        validate_shortcuts(&config.shortcuts)
                            .and_then(|()| validate_terminal_shortcuts(config))
                            .map_err(settings::Error::InvalidShortcuts)?;
                    }
                    Ok(())
                })
            },
            move |result, cx| {
                let result = result
                    .map_err(|error| error.to_string())
                    .and_then(|config| {
                        let saved = PreferenceValues::from(&config);
                        install_preferences(config, cx)?;
                        let _ = owner.update(cx, |owner, cx| {
                            if update_mode {
                                owner.set_view_mode(saved.view.view_mode, cx);
                            }
                            if update_hidden {
                                owner.set_show_hidden_files(saved.view.show_hidden_files, cx);
                            }
                        });
                        Ok(saved)
                    });
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |this, window, cx| {
                        this.saving = false;
                        let submitted_background = this.draft.terminal.background.clone();
                        match result {
                            Ok(saved) => this.baseline = saved,
                            Err(error) => this.error = Some(error),
                        }
                        this.draft = this.baseline.clone();
                        let background = &this.draft.terminal.background;
                        if *background != submitted_background
                            && this.terminal_background.read(cx).value().trim()
                                == submitted_background
                        {
                            this.terminal_background.update(cx, |input, cx| {
                                input.set_value(background.clone(), window, cx);
                            });
                        }
                        cx.notify();
                    });
                }
            },
        );
        cx.notify();
    }

    fn shortcut_value(&mut self, field: ShortcutField) -> &mut String {
        match field {
            ShortcutField::NewTab => &mut self.draft.shortcuts.new_tab,
            ShortcutField::CloseTab => &mut self.draft.shortcuts.close_tab,
            ShortcutField::OpenTerminal => &mut self.draft.shortcuts.open_terminal,
            ShortcutField::Copy => &mut self.draft.terminal.copy_shortcut,
            ShortcutField::Paste => &mut self.draft.terminal.paste_shortcut,
        }
    }

    fn record_shortcut(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.recording else { return };
        if !self.shortcut_focus.is_focused(window) {
            return;
        }
        cx.stop_propagation();
        if event.is_held || self.busy() {
            return;
        }
        let key = &event.keystroke;
        if key.key == "escape" && !key.modifiers.modified() {
            self.recording = None;
            self.error = None;
        } else {
            let value = key.unparse(shortcut_os());
            if let Err(error) = parse_shortcut(&value, field.label()) {
                self.error = Some(error);
                cx.notify();
                return;
            }
            *self.shortcut_value(field) = value;
            self.recording = None;
            window.focus(&self.focus, cx);
            self.save(window, cx);
            return;
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn shortcut_row(
        &self,
        field: ShortcutField,
        value: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let recording = self.recording == Some(field);
        let caption = if recording {
            "按键（esc 取消）".to_owned()
        } else {
            parse_shortcut(value, field.label()).map_or_else(
                |_| value.to_lowercase(),
                |key| {
                    let binding = key.unparse(shortcut_os());
                    let prefix = &binding[..binding.len() - key.key.len()];
                    format!("{}{}", prefix.replace('-', "+"), key.key.to_lowercase())
                },
            )
        };
        preference_row(
            field.label(),
            Button::new(field.label())
                .outline()
                .w(px(144.))
                .h_9()
                .px_3()
                .label(caption)
                .disabled(
                    self.busy()
                        || (matches!(field, ShortcutField::Copy | ShortcutField::Paste)
                            && self.embedded_disabled()),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.busy()
                        || (matches!(field, ShortcutField::Copy | ShortcutField::Paste)
                            && this.embedded_disabled())
                    {
                        return;
                    }
                    this.recording = Some(field);
                    this.error = None;
                    window.focus(&this.shortcut_focus, cx);
                    cx.notify();
                })),
        )
    }

    fn choose_terminal_program(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || self.draft.terminal.mode != TerminalMode::External {
            return;
        }
        self.choosing_program = true;
        self.error = None;
        cx.notify();
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择外部终端程序".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = selection.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.choosing_program = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            if let Some(path) = path.to_str() {
                                this.change(window, cx, |draft| {
                                    draft.terminal.external_program = quote_program_path(path)
                                });
                                return;
                            } else {
                                this.error = Some("程序路径无法转换为文本".into());
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.error = Some(format!("无法打开文件选择器：{error}")),
                    Err(error) => this.error = Some(format!("文件选择未完成：{error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn font_size_control(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let lower = *FONT_SIZE_RANGE.start();
        let upper = *FONT_SIZE_RANGE.end();
        let configured = self.draft.terminal.font_size;
        let displayed_size = if configured.is_finite() {
            configured.clamp(lower, upper)
        } else {
            DEFAULT_FONT_SIZE
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("terminal-font-decrease")
                    .outline()
                    .small()
                    .label("−")
                    .disabled(self.embedded_disabled() || displayed_size <= lower)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.embedded_disabled() {
                            this.change(window, cx, |draft| {
                                draft.terminal.font_size = (displayed_size - 1.).max(lower)
                            });
                        }
                    })),
            )
            .child(
                div()
                    .min_w(px(60.))
                    .text_center()
                    .text_sm()
                    .child(format!("{displayed_size} px")),
            )
            .child(
                Button::new("terminal-font-increase")
                    .outline()
                    .small()
                    .label("+")
                    .disabled(self.embedded_disabled() || displayed_size >= upper)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.embedded_disabled() {
                            this.change(window, cx, |draft| {
                                draft.terminal.font_size = (displayed_size + 1.).min(upper)
                            });
                        }
                    })),
            )
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
                            preference_choice(label, label, self.draft.view.view_mode == mode, cx)
                                .disabled(self.busy())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.change(window, cx, |draft| draft.view.view_mode = mode);
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
                    .checked(self.draft.view.show_hidden_files)
                    .disabled(self.busy())
                    .on_change(cx.listener(|this, checked: &bool, window, cx| {
                        this.change(window, cx, |draft| draft.view.show_hidden_files = *checked);
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
                        preference_choice(id, label, self.draft.blank_double_click == action, cx)
                            .disabled(self.busy())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.change(window, cx, |draft| draft.blank_double_click = action);
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
                        preference_choice(id, label, self.draft.middle_click_tab == action, cx)
                            .disabled(self.busy())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.change(window, cx, |draft| draft.middle_click_tab = action);
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
                        preference_choice(id, label, self.draft.middle_click_folder == action, cx)
                            .disabled(self.busy())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.change(window, cx, |draft| draft.middle_click_folder = action);
                            }))
                    }),
                ),
            ))
    }

    fn terminal_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .gap_3()
            .child(preference_row(
                "快捷键打开",
                preference_segments(
                    cx,
                    [
                        ("embedded-terminal", "内置终端", TerminalMode::Embedded),
                        ("external-terminal", "外部终端", TerminalMode::External),
                    ]
                    .into_iter()
                    .map(|(id, label, mode)| {
                        preference_choice(id, label, self.draft.terminal.mode == mode, cx)
                            .disabled(self.busy())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.change(window, cx, |draft| draft.terminal.mode = mode);
                            }))
                    }),
                ),
            ))
            .child(preference_row(
                "光标形状",
                preference_segments(
                    cx,
                    [
                        ("terminal-cursor-block", "方块", TerminalCursorShape::Block),
                        ("terminal-cursor-beam", "竖线", TerminalCursorShape::Beam),
                        (
                            "terminal-cursor-underline",
                            "下划线",
                            TerminalCursorShape::Underline,
                        ),
                    ]
                    .into_iter()
                    .map(|(id, label, shape)| {
                        preference_choice(id, label, self.draft.terminal.cursor_shape == shape, cx)
                            .disabled(self.embedded_disabled())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if !this.embedded_disabled() {
                                    this.change(window, cx, |draft| {
                                        draft.terminal.cursor_shape = shape
                                    });
                                }
                            }))
                    }),
                ),
            ))
            .child(preference_row(
                "背景颜色",
                div().w(px(176.)).child(
                    Input::new(&self.terminal_background).disabled(self.embedded_disabled()),
                ),
            ))
            .child(preference_row("字号", self.font_size_control(cx)))
            .child(self.shortcut_row(ShortcutField::Copy, &self.draft.terminal.copy_shortcut, cx))
            .child(self.shortcut_row(
                ShortcutField::Paste,
                &self.draft.terminal.paste_shortcut,
                cx,
            ))
            .child(self.external_program_row(cx))
    }

    fn external_program_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let program = &self.draft.terminal.external_program;
        let disabled = self.busy() || self.draft.terminal.mode != TerminalMode::External;
        let automatic = program.is_empty();
        let program_path = sys::opener::split_command(program).map(|(name, _)| name);
        let modes = preference_segments(
            cx,
            [
                preference_choice("automatic-terminal-program", "自动探测", automatic, cx)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.draft.terminal.mode == TerminalMode::External {
                            this.change(window, cx, |draft| {
                                draft.terminal.external_program.clear()
                            });
                        }
                    })),
                preference_choice("browse-terminal-program", "指定程序…", !automatic, cx)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.choose_terminal_program(window, cx);
                    })),
            ],
        );
        v_flex()
            .gap_1()
            .child(preference_row("外部终端程序", modes))
            .when_some(program_path, |row, path| {
                row.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_right()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(path),
                )
            })
    }

    fn shortcuts_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .gap_3()
            .child(self.shortcut_row(ShortcutField::NewTab, &self.draft.shortcuts.new_tab, cx))
            .child(self.shortcut_row(ShortcutField::CloseTab, &self.draft.shortcuts.close_tab, cx))
            .child(self.shortcut_row(
                ShortcutField::OpenTerminal,
                &self.draft.shortcuts.open_terminal,
                cx,
            ))
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
            Page::Terminal => self.terminal_page(cx).into_any_element(),
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
            .capture_key_down(cx.listener(Self::record_shortcut))
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .id("shortcut-recorder-focus")
                    .absolute()
                    .track_focus(&self.shortcut_focus),
            )
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
                            .when(self.saving || self.error.is_some(), |panel| {
                                let message =
                                    self.error.clone().unwrap_or_else(|| "正在保存…".into());
                                panel.child(
                                    div()
                                        .flex_shrink_0()
                                        .px_6()
                                        .py_3()
                                        .border_t_1()
                                        .border_color(palette.border)
                                        .text_sm()
                                        .text_color(if self.error.is_some() {
                                            danger
                                        } else {
                                            foreground
                                        })
                                        .child(message),
                                )
                            }),
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
        .items_center()
        .justify_between()
        .gap_3()
        .child(div().text_sm().child(label))
        .child(control)
}

pub(super) fn shortcut_bindings(
    shortcuts: &ShortcutConfig,
    cx: &gpui_kit::App,
) -> Result<[KeyBinding; 3], String> {
    validate_shortcuts(shortcuts)?;
    Ok([
        key_binding(&shortcuts.new_tab, NewTab, false, cx)?,
        key_binding(&shortcuts.close_tab, CloseTab, false, cx)?,
        key_binding(&shortcuts.open_terminal, OpenTerminal, true, cx)?,
    ])
}

fn shortcut_os() -> OperatingSystem {
    if cfg!(target_os = "macos") {
        OperatingSystem::Mac
    } else if cfg!(target_os = "windows") {
        OperatingSystem::Windows
    } else if cfg!(target_os = "freebsd") {
        OperatingSystem::FreeBsd
    } else {
        OperatingSystem::Linux
    }
}

fn quote_program_path(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

pub(super) fn parse_shortcut(value: &str, label: &str) -> Result<Keystroke, String> {
    let key = Keystroke::parse_with_os(value, shortcut_os())
        .map_err(|error| format!("{label}的快捷键无效：{error}"))?;
    if value.split_whitespace().count() != 1
        || key.key.is_empty()
        || key.key_char.is_some()
        || !(key.modifiers.control || key.modifiers.alt || key.modifiers.platform)
    {
        return Err(format!("{label}需要一个包含 Ctrl、Alt 或 Super 的组合键"));
    }
    Ok(key)
}

fn validate_terminal_shortcuts(config: &AppConfig) -> Result<(), String> {
    let copy = parse_shortcut(&config.terminal.copy_shortcut, "内置终端复制")?;
    let paste = parse_shortcut(&config.terminal.paste_shortcut, "内置终端粘贴")?;
    let open = parse_shortcut(&config.shortcuts.open_terminal, "打开终端")?;
    if copy == paste || copy == open || paste == open {
        return Err("打开终端、内置终端复制和粘贴需要使用不同的快捷键".into());
    }
    Ok(())
}

fn validate_shortcuts(shortcuts: &ShortcutConfig) -> Result<(), String> {
    let parse = parse_shortcut;
    let new = parse(&shortcuts.new_tab, "新建标签页")?;
    let close = parse(&shortcuts.close_tab, "关闭标签页")?;
    let terminal = parse(&shortcuts.open_terminal, "打开终端")?;
    let preferences = parse("ctrl-p", "偏好设置")?;
    if new == preferences || close == preferences || terminal == preferences {
        return Err("Ctrl+P 已用于打开偏好设置".into());
    }
    if new == close || terminal == new || terminal == close {
        return Err("两个操作不能使用相同的快捷键".into());
    }
    Ok(())
}

fn key_binding(
    value: &str,
    action: impl Action,
    in_terminal: bool,
    cx: &gpui_kit::App,
) -> Result<KeyBinding, String> {
    let context = if in_terminal {
        "FileManager"
    } else {
        "FileManager && !EmbeddedTerminal"
    };
    let context = KeyBindingContextPredicate::parse(context).map_err(|error| error.to_string())?;
    KeyBinding::load(
        value,
        Box::new(action),
        Some(context.into()),
        false,
        None,
        // gpui-kit 0.10 起 load 直接收 &App，内部自行解析键盘映射。
        cx,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        AppConfig, BlankDoubleClickAction, Keystroke, PreferenceValues, ShortcutConfig, ViewConfig,
        ViewMode, parse_shortcut, quote_program_path, shortcut_os,
    };

    #[test]
    fn recorded_shortcuts_round_trip_with_modifiers() {
        for source in ["ctrl-shift-c", "alt-f12", "super-v", "ctrl-`"] {
            let event = Keystroke::parse_with_os(source, shortcut_os()).unwrap();
            let stored = event.unparse(shortcut_os());
            assert_eq!(parse_shortcut(&stored, "测试").unwrap(), event);
        }
    }

    #[test]
    fn selected_program_paths_round_trip_through_command_parser() {
        let path = "/opt/Terminal's files/term\name";
        let command = quote_program_path(path);
        assert_eq!(
            sys::opener::split_command(&command),
            Some((path.to_owned(), vec![]))
        );
    }

    #[test]
    fn preference_draft_does_not_overwrite_unedited_fields() {
        let baseline = PreferenceValues {
            shortcuts: ShortcutConfig::default(),
            terminal: Default::default(),
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
