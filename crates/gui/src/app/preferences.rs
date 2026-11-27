use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, Selectable as _};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::action::{
    BlankDoubleClickAction, FolderMiddleClickAction, TabMiddleClickAction,
};
use settings::config::shortcuts::ShortcutConfig;
use settings::config::terminal::{
    DEFAULT_FONT_SIZE, FONT_SIZE_RANGE, TerminalConfig, TerminalCursorShape, TerminalMode,
};
use settings::config::view::{FileDateFormat, ViewMode};
use settings::config::{AppConfig, ViewConfig};

use super::embed_terminal::parse_background;
use super::io_queue::IoQueue;
use super::keymap::{
    SHORTCUTS, ShortcutGroup, ShortcutScope, bind_all, parse_shortcut, shortcut_os, unbind_all,
    validate,
};
use super::{App, window_resize};
use crate::components::preferences_title_bar;
use crate::style::{APP_ID, WINDOW_RADIUS, preferences_palette};

pub(super) struct ActivePreferences(pub AppConfig);

impl Global for ActivePreferences {}

const PREFERENCES_SIDEBAR_WIDTH: Pixels = px(176.);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    View,
    Behavior,
    Shortcuts,
    Terminal,
    #[cfg(target_os = "linux")]
    Cache,
    About,
}

impl Page {
    const ALL: &[Self] = &[
        Self::View,
        Self::Behavior,
        Self::Terminal,
        Self::Shortcuts,
        #[cfg(target_os = "linux")]
        Self::Cache,
        Self::About,
    ];

    fn appearance(self) -> (&'static str, IconName) {
        match self {
            Self::View => ("文件视图", IconName::LayoutGrid),
            Self::Behavior => ("操作行为", IconName::Mouse),
            Self::Terminal => ("终端", IconName::Terminal),
            Self::Shortcuts => ("快捷键", IconName::Keyboard),
            #[cfg(target_os = "linux")]
            Self::Cache => ("缓存", IconName::Database),
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
    /// 把草稿中用户改过的字段合并进磁盘配置。
    ///
    /// 只写草稿与基线的差异：保存前其他进程写入的新值不会被打开
    /// 设置窗口时的旧快照覆盖。快捷键逐字段比较由 `SHORTCUTS`
    /// 注册表驱动，新增字段无需改这里。
    fn merge_into(self, baseline: Self, config: &mut AppConfig) {
        for entry in SHORTCUTS {
            let (draft, base) = (
                (entry.get)(&self.shortcuts),
                (entry.get)(&baseline.shortcuts),
            );
            if draft != base {
                *(entry.get_mut)(&mut config.shortcuts) = draft.to_owned();
            }
        }
        if self.terminal.mode != baseline.terminal.mode {
            config.terminal.mode = self.terminal.mode;
        }
        if self.terminal.external_program != baseline.terminal.external_program {
            config.terminal.external_program = self.terminal.external_program;
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
        if self.view.remember_tab_state != baseline.view.remember_tab_state {
            config.view.remember_tab_state = self.view.remember_tab_state;
        }
        if self.view.file_date_format != baseline.view.file_date_format {
            config.view.file_date_format = self.view.file_date_format;
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

#[cfg(target_os = "linux")]
enum CacheStatus {
    Idle,
    Reading,
    Clearing,
    Usage(u64),
    Failed(String),
}

/// 设置窗口持有自己的草稿，只有保存成功才更新文件管理器。
pub(super) struct Preferences {
    owner: WeakEntity<App>,
    baseline: PreferenceValues,
    saving: bool,
    focus: FocusHandle,
    page: Page,
    draft: PreferenceValues,
    /// 正在录制的快捷键，指向 `keymap::SHORTCUTS` 的下标。
    recording: Option<usize>,
    shortcut_focus: FocusHandle,
    /// 快捷键页顶部的搜索词。
    shortcut_search: Entity<InputState>,
    /// 按键查找模式：下一次按键写入搜索框而不是当作输入。
    search_capture: bool,
    /// 录制/按键查找时当前按住的修饰键，用于录制槽里的实时预览。
    held_modifiers: Vec<&'static str>,
    choosing_program: bool,
    terminal_background: Entity<InputState>,
    #[cfg(target_os = "linux")]
    cache_status: CacheStatus,
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
                        this.search_capture = false;
                        this.held_modifiers.clear();
                        if this.recording.take().is_some() {
                            cx.notify();
                        }
                    },
                )
                .detach();
                let terminal_background = cx.new(|cx| {
                    InputState::new(window, cx).default_value(baseline.terminal.background.clone())
                });
                let shortcut_search = cx.new(|cx| {
                    InputState::new(window, cx).placeholder("搜索名称，或直接按下组合键")
                });
                cx.subscribe_in(
                    &shortcut_search,
                    window,
                    |_: &mut Preferences,
                     _: &Entity<InputState>,
                     event: &InputEvent,
                     _: &mut Window,
                     cx: &mut Context<Preferences>| {
                        if matches!(event, InputEvent::Change) {
                            cx.notify();
                        }
                    },
                )
                .detach();
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
                    shortcut_search,
                    shortcut_focus,
                    search_capture: false,
                    recording: None,
                    held_modifiers: Vec::new(),
                    choosing_program: false,
                    terminal_background,
                    #[cfg(target_os = "linux")]
                    cache_status: CacheStatus::Idle,
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
    if config.shortcuts != cx.global::<ActivePreferences>().0.shortcuts {
        validate(&config.shortcuts)?;
        // 先解绑旧键再绑新键，避免同一动作同时挂着两个键。
        // 校验通过后再改动当前窗口的键位。
        let old = cx.global::<ActivePreferences>().0.shortcuts.clone();
        unbind_all(&old, cx);
        bind_all(&config.shortcuts, cx)?;
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
                AppConfig::update_file(|config| {
                    draft.merge_into(baseline, config);
                    validate(&config.shortcuts).map_err(settings::Error::InvalidShortcuts)
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

    /// 快捷键页的统一按键入口：录制改键、按键查找，以及搜索框聚焦时
    /// 直接按下组合键查找。修饰键的实时预览由 [`Self::update_held_modifiers`] 维护。
    fn handle_shortcut_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        let trap_focused = self.shortcut_focus.is_focused(window);
        let input_focused = self
            .shortcut_search
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        if !trap_focused && !input_focused {
            return;
        }
        // 纯修饰键的按下/抬起不结束捕获，预览交给 modifiers 变化回调。
        if key.is_modifier_key() {
            if trap_focused {
                cx.stop_propagation();
            }
            return;
        }

        // 焦点在捕获层：录制/查找期间所有按键在此终结，
        // 绝不冒泡给根节点的“Esc 关闭窗口”。
        if trap_focused {
            if !self.search_capture && self.recording.is_none() {
                // 空闲的捕获层：Esc 先退回主焦点（再次按 Esc 才可能关窗），
                // 其余按键放行。
                if key.key == "escape" && !key.modifiers.modified() {
                    cx.stop_propagation();
                    window.focus(&self.focus, cx);
                }
                return;
            }
            cx.stop_propagation();
            if event.is_held {
                return;
            }

            // 按键查找：把按下的任意按键写入搜索框，Esc 取消。
            if self.search_capture {
                if key.key == "escape" && !key.modifiers.modified() {
                    self.search_capture = false;
                } else {
                    let value = key.unparse(shortcut_os());
                    self.shortcut_search
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                    self.search_capture = false;
                }
                self.held_modifiers.clear();
                self.focus_after_capture(window, cx);
                cx.notify();
                return;
            }

            // 录制改键。
            let Some(index) = self.recording else { return };
            let Some(entry) = SHORTCUTS.get(index) else {
                return;
            };
            if self.busy() {
                return;
            }
            self.held_modifiers.clear();
            if key.key == "escape" && !key.modifiers.modified() {
                self.recording = None;
                self.error = None;
            } else if key.key == "backspace" && !key.modifiers.modified() {
                // 退格清除绑定，空串即未绑定。
                *((entry.get_mut)(&mut self.draft.shortcuts)) = String::new();
                self.recording = None;
                self.focus_after_capture(window, cx);
                self.save(window, cx);
                return;
            } else {
                let value = key.unparse(shortcut_os());
                let mut shortcuts = self.draft.shortcuts.clone();
                *(entry.get_mut)(&mut shortcuts) = value;
                if let Err(error) = validate(&shortcuts) {
                    // 无效按键只提示不退出录制，直接重按即可。
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
                self.draft.shortcuts = shortcuts;
                self.recording = None;
                self.focus_after_capture(window, cx);
                self.save(window, cx);
                return;
            }
        }

        // 搜索框聚焦时，输入框自己未占用的组合键与功能键直接转为按键查找。
        // 输入框占用的键（Ctrl-A、退格、方向键等）在键绑定阶段就被消费，到不了这里。
        if !event.is_held && is_direct_search_key(key) {
            cx.stop_propagation();
            let value = key.unparse(shortcut_os());
            self.shortcut_search
                .update(cx, |input, cx| input.set_value(value, window, cx));
            cx.notify();
        }
    }

    /// 结束录制/按键查找后的落点：快捷键页回到搜索框（此后 Esc 只会清空
    /// 搜索词）；其他页面留在捕获层，下一次 Esc 先退回主焦点而不是直接关窗。
    fn focus_after_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Shortcuts {
            let search = self.shortcut_search.read(cx).focus_handle(cx);
            window.focus(&search, cx);
        }
    }

    /// 录制与按键查找时刷新按住的修饰键预览。
    fn update_held_modifiers(
        &mut self,
        event: &ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.recording.is_none() && !self.search_capture {
            return;
        }
        let labels = held_modifier_labels(&event.modifiers);
        if self.held_modifiers != labels {
            self.held_modifiers = labels;
            cx.notify();
        }
    }

    fn shortcut_row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entry = &SHORTCUTS[index];
        let value = (entry.get)(&self.draft.shortcuts);
        let recording = self.recording == Some(index);
        let disabled =
            self.busy() || (entry.scope == ShortcutScope::Terminal && self.embedded_disabled());
        let theme = cx.theme();
        let border = preferences_palette(theme.mode).border;
        // 录制中：单行录制槽。accent 只用在边框上，文字与键帽用常规配色，
        // 完整操作提示放 tooltip。
        let content = if recording {
            h_flex()
                .flex_shrink_0()
                .w_full()
                .h(px(28.))
                .px(px(8.))
                .gap_1()
                .items_center()
                .rounded(px(5.))
                .border_1()
                .border_color(theme.accent)
                .bg(theme.accent.opacity(0.08))
                .text_xs()
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .items_center()
                        .when(self.held_modifiers.is_empty(), |row| {
                            row.child(div().child("按下新按键…"))
                        })
                        .when(!self.held_modifiers.is_empty(), |row| {
                            row.children(
                                self.held_modifiers
                                    .iter()
                                    .map(|label| keycap(label, border, theme.list_hover)),
                            )
                            .child(div().text_color(theme.muted_foreground).child("+"))
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .child("Esc 取消"),
                )
                .into_any_element()
        } else {
            match parse_shortcut(value, entry.label) {
                Ok(key) if !value.trim().is_empty() => h_flex()
                    .items_center()
                    .gap_1()
                    .children(
                        keycap_labels(&key)
                            .into_iter()
                            .map(|label| keycap(&label, border, theme.list_hover)),
                    )
                    .into_any_element(),
                _ if value.trim().is_empty() => div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("未绑定")
                    .into_any_element(),
                _ => div()
                    .text_xs()
                    .child(value.to_lowercase())
                    .into_any_element(),
            }
        };
        // 整块仍是可点击组件，但不再用边框包裹，悬停以底色反馈。
        let key_area = h_flex()
            .id(SharedString::from(format!("shortcut-{index}")))
            .role(Role::Button)
            .aria_label(entry.label)
            .justify_end()
            .items_center()
            .w(px(190.))
            .h_9()
            .px_2()
            .rounded_md()
            .when(disabled, |row| row.opacity(0.5))
            .when(!disabled, |row| {
                row.cursor_pointer()
                    .hover(|style| style.bg(theme.list_hover.opacity(0.6)))
            })
            .when(recording, |row| {
                row.tooltip(|window, cx| Tooltip::new("Esc 取消，退格清除绑定").build(window, cx))
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.busy()
                    || (SHORTCUTS[index].scope == ShortcutScope::Terminal
                        && this.embedded_disabled())
                {
                    return;
                }
                this.search_capture = false;
                this.held_modifiers.clear();
                this.recording = Some(index);
                this.error = None;
                window.focus(&this.shortcut_focus, cx);
                cx.notify();
            }))
            .child(content);
        preference_row(entry.label, key_area)
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
            .children(Page::ALL.iter().copied().map(|page| {
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
                        #[cfg(target_os = "linux")]
                        if page == Page::Cache {
                            this.update_thumbnail_cache(false, cx);
                        }
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
            .child(self.view_switch(
                "hidden-files",
                "显示隐藏文件",
                self.draft.view.show_hidden_files,
                |view, checked| view.show_hidden_files = checked,
                cx,
            ))
            .child(self.view_switch(
                "remember-tab-state",
                "切换标签页时保留浏览状态",
                self.draft.view.remember_tab_state,
                |view, checked| view.remember_tab_state = checked,
                cx,
            ))
            .child(preference_row(
                "日期和时间格式",
                preference_segments(
                    cx,
                    [
                        (FileDateFormat::Simple, "简单"),
                        (FileDateFormat::Detailed, "详细"),
                    ]
                    .into_iter()
                    .map(|(format, label)| {
                        preference_choice(
                            label,
                            label,
                            self.draft.view.file_date_format == format,
                            cx,
                        )
                        .disabled(self.busy())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.change(window, cx, |draft| {
                                    draft.view.file_date_format = format
                                });
                            },
                        ))
                    }),
                ),
            ))
    }

    fn view_switch(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        update: fn(&mut ViewConfig, bool),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        preference_row(
            label,
            Switch::new(id)
                .accessibility_label(label)
                .color(rgb(if cx.theme().mode.is_dark() {
                    0x767676
                } else {
                    0x454340
                }))
                .checked(checked)
                .disabled(self.busy())
                .on_change(cx.listener(move |this, checked: &bool, window, cx| {
                    this.change(window, cx, |draft| update(&mut draft.view, *checked));
                })),
        )
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
            .children(
                SHORTCUTS
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| entry.scope == ShortcutScope::Terminal)
                    .map(|(index, _)| self.shortcut_row(index, cx)),
            )
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
        let query = normalized_shortcut_query(&self.shortcut_search.read(cx).value());
        let muted = cx.theme().muted_foreground;
        let border = preferences_palette(cx.theme().mode).border;
        let mut matched = 0;
        let mut sections = Vec::new();
        for group in [
            ShortcutGroup::Tabs,
            ShortcutGroup::Files,
            ShortcutGroup::View,
            ShortcutGroup::Terminal,
            ShortcutGroup::App,
        ] {
            let mut rows = Vec::new();
            for (index, entry) in SHORTCUTS.iter().enumerate() {
                if entry.group == group
                    && shortcut_matches(entry, &self.draft.shortcuts, query.as_deref())
                {
                    matched += 1;
                    rows.push(self.shortcut_row(index, cx));
                }
            }
            let count = rows.len();
            if count > 0 {
                // 每组一张卡片，卡内行间用分隔线，避免上下串行。
                let card = v_flex()
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .overflow_hidden()
                    .children(rows.into_iter().enumerate().map(|(position, row)| {
                        div()
                            .px_3()
                            .when(position + 1 < count, |row| {
                                row.border_b_1().border_color(border)
                            })
                            .child(row)
                    }));
                sections.push(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .text_color(muted)
                                .child(group.label()),
                        )
                        .child(card),
                );
            }
        }
        let capturing = self.search_capture;
        let (accent, muted) = (cx.theme().accent, cx.theme().muted_foreground);
        // 顶部查找框：输入文字按名称过滤；聚焦时直接按下 Ctrl/Alt/Super 组合
        // 或功能键即按按键查找。键盘图标开启按键查找模式，可捕获 Esc、Home 等裸键。
        // accent 只用在边框和图标上，文字用常规前景色保证可读。
        let search_field: AnyElement = if capturing {
            h_flex()
                .id("shortcut-search-capture")
                .role(Role::Button)
                .aria_label("取消按键查找")
                .w_full()
                .h(px(32.))
                .px_3()
                .gap_2()
                .items_center()
                .rounded_md()
                .border_1()
                .border_color(accent)
                .bg(accent.opacity(0.06))
                .cursor_pointer()
                .hover(|style| style.bg(accent.opacity(0.1)))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.search_capture = false;
                    this.held_modifiers.clear();
                    window.focus(&this.focus, cx);
                    cx.notify();
                }))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .items_center()
                        .text_sm()
                        .when(self.held_modifiers.is_empty(), |row| {
                            row.child(div().child("按下要查找的按键…"))
                        })
                        .when(!self.held_modifiers.is_empty(), |row| {
                            row.children(
                                self.held_modifiers
                                    .iter()
                                    .map(|label| keycap(label, border, cx.theme().list_hover)),
                            )
                            .child(div().text_color(muted).child("+"))
                        }),
                )
                .child(div().text_xs().text_color(muted).child("Esc 取消"))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .child(
                    Input::new(&self.shortcut_search)
                        .prefix(Icon::new(IconName::Search).size(px(14.)).text_color(muted))
                        .suffix(
                            h_flex()
                                .id("shortcut-search-capture-toggle")
                                .role(Role::Button)
                                .aria_label("按键查找")
                                .cursor_pointer()
                                .p(px(3.))
                                .rounded(px(4.))
                                .hover(|style| style.bg(cx.theme().list_hover))
                                .child(
                                    Icon::new(IconName::Keyboard)
                                        .size(px(14.))
                                        .text_color(muted),
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    // 进入按键查找：焦点移到捕获层，下一次按键写入搜索框。
                                    this.search_capture = true;
                                    this.recording = None;
                                    this.held_modifiers.clear();
                                    window.focus(&this.shortcut_focus, cx);
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element()
        };
        v_flex()
            .gap_4()
            .child(h_flex().gap_2().child(search_field))
            .children(sections)
            .when(query.is_some() && matched == 0, |page| {
                page.child(div().text_sm().text_color(muted).child("没有匹配的快捷键"))
            })
    }

    #[cfg(target_os = "linux")]
    fn update_thumbnail_cache(&mut self, clear: bool, cx: &mut Context<Self>) {
        if matches!(
            self.cache_status,
            CacheStatus::Reading | CacheStatus::Clearing
        ) {
            return;
        }
        self.cache_status = if clear {
            CacheStatus::Clearing
        } else {
            CacheStatus::Reading
        };
        let work = cx.background_spawn(async move {
            let cache = sys::thumbnail_cache::ThumbnailCache::open()
                .ok_or_else(|| std::io::Error::other("找不到缓存目录"))?;
            if clear {
                cache.clear()
            } else {
                cache.disk_usage()
            }
        });
        cx.spawn(async move |preferences, cx| {
            let result = work.await;
            let _ = preferences.update(cx, |preferences, cx| {
                preferences.cache_status = match result {
                    Ok(bytes) => CacheStatus::Usage(bytes),
                    Err(error) => CacheStatus::Failed(error.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[cfg(target_os = "linux")]
    fn cache_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (description, error) = match &self.cache_status {
            CacheStatus::Idle | CacheStatus::Reading => ("正在读取…".into(), false),
            CacheStatus::Clearing => ("正在清理…".into(), false),
            CacheStatus::Usage(bytes) => (super::file_area::format_size(*bytes), false),
            CacheStatus::Failed(error) => (error.clone(), true),
        };
        let theme = cx.theme();
        let detail_color = if error {
            theme.danger
        } else {
            theme.muted_foreground
        };
        preference_row(
            "ktfm 磁盘缩略图缓存",
            h_flex()
                .gap_3()
                .child(div().text_sm().text_color(detail_color).child(description))
                .child(
                    Button::new("clear-thumbnail-cache")
                        .label("清理缓存")
                        .small()
                        .disabled(matches!(
                            self.cache_status,
                            CacheStatus::Idle
                                | CacheStatus::Reading
                                | CacheStatus::Clearing
                                | CacheStatus::Usage(0)
                        ))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.update_thumbnail_cache(true, cx)),
                        ),
                ),
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
        let font_family = theme.font_family.clone();
        let (foreground, danger) = (theme.foreground, theme.danger);
        let (title, _) = self.page.appearance();
        let content = match self.page {
            Page::View => self.view_page(cx).into_any_element(),
            Page::Behavior => self.behavior_page(cx).into_any_element(),
            Page::Terminal => self.terminal_page(cx).into_any_element(),
            Page::Shortcuts => self.shortcuts_page(cx).into_any_element(),
            #[cfg(target_os = "linux")]
            Page::Cache => self.cache_page(cx).into_any_element(),
            Page::About => self.about_page(cx).into_any_element(),
        };

        v_flex()
            .id("preferences-window")
            .font_family(font_family)
            .relative()
            .size_full()
            .rounded(radius)
            .bg(palette.background)
            .text_color(foreground)
            .when(!square, |frame| {
                frame.border_1().border_color(palette.border)
            })
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::handle_shortcut_key))
            // 录制与按键查找时刷新按住的修饰键，用于录制槽的实时预览。
            .on_modifiers_changed(cx.listener(Self::update_held_modifiers))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key != "escape" {
                    return;
                }
                // Esc 逐层退让：录制、按键查找、保存进行中绝不关闭窗口。
                if this.recording.is_some() || this.search_capture || this.saving {
                    cx.stop_propagation();
                    return;
                }
                // 搜索框聚焦时 Esc 只清空搜索词，不关闭窗口。
                if this
                    .shortcut_search
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
                {
                    if !this.shortcut_search.read(cx).value().trim().is_empty() {
                        this.shortcut_search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    cx.stop_propagation();
                    return;
                }
                // 有搜索词时 Esc 先清空搜索，再次按下才关闭窗口。
                if !this.shortcut_search.read(cx).value().trim().is_empty() {
                    this.shortcut_search
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    cx.stop_propagation();
                    return;
                }
                // 有错误提示时先关掉提示，再按才关闭窗口。
                if this.error.is_some() {
                    this.error = None;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                window.remove_window();
                cx.stop_propagation();
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

/// 键帽展示顺序与文本：修饰键在前，主键在后。
fn keycap_labels(key: &Keystroke) -> Vec<String> {
    let mut labels = Vec::new();
    if key.modifiers.control {
        labels.push("Ctrl".into());
    }
    if key.modifiers.alt {
        labels.push("Alt".into());
    }
    if key.modifiers.shift {
        labels.push("Shift".into());
    }
    if key.modifiers.platform {
        labels.push(
            if cfg!(target_os = "macos") {
                "Cmd"
            } else {
                "Super"
            }
            .into(),
        );
    }
    labels.push(display_key_name(&key.key));
    labels
}

/// 主键的展示名：单键大写，功能键与方向键用习惯缩写。
fn display_key_name(key: &str) -> String {
    match key {
        "escape" => "Esc".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Del".into(),
        "insert" => "Ins".into(),
        "enter" => "Enter".into(),
        "tab" => "Tab".into(),
        "space" => "Space".into(),
        "up" => "Up".into(),
        "down" => "Down".into(),
        "left" => "Left".into(),
        "right" => "Right".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PgUp".into(),
        "pagedown" => "PgDn".into(),
        single if single.chars().count() == 1 => single.to_uppercase(),
        function
            if function.starts_with('f') && function[1..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            function.to_uppercase()
        }
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    }
}

/// 单个键帽格：底色填充，比仅边框更醒目。
fn keycap(label: &str, border: Hsla, surface: Hsla) -> impl IntoElement + use<> {
    h_flex()
        .flex_shrink_0()
        .min_w(px(22.))
        .h(px(22.))
        .px(px(6.))
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .border_1()
        .border_color(border)
        .bg(surface)
        .text_xs()
        .child(label.to_owned())
}

/// 录制之外的展示文案：未绑定或规范化按键（如 ctrl+shift+n）。
fn shortcut_caption(entry: &super::keymap::ShortcutEntry, value: &str) -> String {
    if value.trim().is_empty() {
        "未绑定".to_owned()
    } else {
        parse_shortcut(value, entry.label).map_or_else(
            |_| value.to_lowercase(),
            |key| {
                let binding = key.unparse(shortcut_os());
                let prefix = &binding[..binding.len() - key.key.len()];
                format!("{}{}", prefix.replace('-', "+"), key.key.to_lowercase())
            },
        )
    }
}

/// 搜索词规范化：小写、去空格，并统一按 “ctrl-c” 形式比较按键段。
fn normalized_shortcut_query(value: &str) -> Option<String> {
    let normalized = value
        .trim()
        .to_lowercase()
        .replace(' ', "")
        .replace('+', "-");
    (!normalized.is_empty()).then_some(normalized)
}

/// 搜索框聚焦时可直接转为按键查找的按键：Ctrl/Alt/Super 组合与 F 功能键。
/// 这些按键不会向输入框插入文本；输入框自己占用的键（Ctrl-A、退格、
/// 方向键等）在键绑定阶段就被消费，根本到不了这里。
fn is_direct_search_key(key: &Keystroke) -> bool {
    !key.is_modifier_key()
        && (key.modifiers.control
            || key.modifiers.alt
            || key.modifiers.platform
            || is_function_key(&key.key))
}

/// 是否 F1–F35 这类功能键。
fn is_function_key(key: &str) -> bool {
    key.len() >= 2 && key.starts_with('f') && key[1..].bytes().all(|b| b.is_ascii_digit())
}

/// 按住修饰键的展示顺序，与键帽一致：修饰键在前。
fn held_modifier_labels(modifiers: &Modifiers) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if modifiers.control {
        labels.push("Ctrl");
    }
    if modifiers.alt {
        labels.push("Alt");
    }
    if modifiers.shift {
        labels.push("Shift");
    }
    if modifiers.platform {
        labels.push(if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Super"
        });
    }
    labels
}

/// 名称或当前按键包含搜索词即命中；无搜索词时全部命中。
fn shortcut_matches(
    entry: &super::keymap::ShortcutEntry,
    shortcuts: &settings::config::shortcuts::ShortcutConfig,
    query: Option<&str>,
) -> bool {
    let Some(query) = query else {
        return true;
    };
    if entry.label.to_lowercase().contains(query) {
        return true;
    }
    let caption = shortcut_caption(entry, (entry.get)(shortcuts));
    caption.to_lowercase().replace('+', "-").contains(query)
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

fn quote_program_path(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::super::keymap::{parse_shortcut, shortcut_os};
    use super::{display_key_name, keycap_labels};

    #[test]
    fn keycap_labels_place_modifiers_before_the_key() {
        let key = parse_shortcut("ctrl-shift-t", "测试").unwrap();
        assert_eq!(keycap_labels(&key), ["Ctrl", "Shift", "T"]);
        let key = parse_shortcut("alt-f2", "测试").unwrap();
        assert_eq!(keycap_labels(&key), ["Alt", "F2"]);
        assert_eq!(display_key_name("pageup"), "PgUp");
        assert_eq!(display_key_name("escape"), "Esc");
        assert_eq!(display_key_name("single"), "Single");
    }

    use super::{
        AppConfig, BlankDoubleClickAction, Keystroke, Modifiers, PreferenceValues, ShortcutConfig,
        ViewConfig, ViewMode, quote_program_path,
    };

    #[test]
    fn direct_search_keys_cover_combos_and_function_keys_only() {
        let key = |source: &str| Keystroke::parse_with_os(source, shortcut_os()).unwrap();
        assert!(super::is_direct_search_key(&key("ctrl-shift-n")));
        assert!(super::is_direct_search_key(&key("alt-f2")));
        assert!(super::is_direct_search_key(&key("super-e")));
        // 裸功能键不插入文本，可直接查找。
        assert!(super::is_direct_search_key(&key("f5")));
        // 裸字母与 Shift 组合仍是正常打字，不触发按键查找。
        assert!(!super::is_direct_search_key(&key("a")));
        assert!(!super::is_direct_search_key(&key("shift-a")));
        // 纯修饰键不结束捕获。
        assert!(!super::is_direct_search_key(&key("control")));
        assert!(super::is_function_key("f12"));
        assert!(!super::is_function_key("f"));
        assert!(!super::is_function_key("filter"));
    }

    #[test]
    fn held_modifier_labels_follow_keycap_order() {
        let modifiers = Modifiers {
            control: true,
            shift: true,
            platform: true,
            ..Modifiers::default()
        };
        let expected: &[&str] = if cfg!(target_os = "macos") {
            &["Ctrl", "Shift", "Cmd"]
        } else {
            &["Ctrl", "Shift", "Super"]
        };
        assert_eq!(super::held_modifier_labels(&modifiers), expected);
        assert!(super::held_modifier_labels(&Modifiers::default()).is_empty());
    }

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
