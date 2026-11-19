use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use daemon::{
    files,
    job::{FileJob, JobKind, JobResult},
};
use gpui_kit::base::{Dialog, Disableable as _};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use vfs::EntryKind;

use super::{App, browser::DirectoryState};
use crate::components::popup_surface;

/// 文件操作命令。既是右键菜单的载荷，也是键绑定的动作：
/// 键盘与菜单共用 [`App::file_action`] 一个分发入口。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FileAction {
    NewFolder,
    NewDocument,
    SelectAll,
    Properties,
    Rename,
    Cut,
    Copy,
    Paste,
    Trash,
}

/// 手工实现 [`Action`]：动作名按变体区分，供 `Unbind` 按名解绑。
/// 不支持从 JSON 构建（本应用不用 JSON 键位表）。
impl Action for FileAction {
    fn boxed_clone(&self) -> Box<dyn Action> {
        Box::new(*self)
    }

    fn partial_eq(&self, action: &dyn Action) -> bool {
        action
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| other == self)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::NewFolder => "file_manager::NewFolder",
            Self::NewDocument => "file_manager::NewDocument",
            Self::SelectAll => "file_manager::SelectAll",
            Self::Properties => "file_manager::Properties",
            Self::Rename => "file_manager::Rename",
            Self::Cut => "file_manager::Cut",
            Self::Copy => "file_manager::Copy",
            Self::Paste => "file_manager::Paste",
            Self::Trash => "file_manager::Trash",
        }
    }

    fn name_for_type() -> &'static str {
        "file_manager::FileAction"
    }

    fn build(_: serde_json::Value) -> gpui_kit::Result<Box<dyn Action>> {
        Err(anyhow::anyhow!("FileAction 不支持按名称构建"))
    }
}

#[derive(Default)]
pub(super) struct FileClipboard {
    paths: Arc<[PathBuf]>,
    cut: bool,
    revision: u64,
    pasting: bool,
}

impl Global for FileClipboard {}

impl FileClipboard {
    pub(super) fn can_cancel_cut(&self) -> bool {
        !self.pasting && !self.pending_cut_paths().is_empty()
    }

    pub(super) fn cancel_cut(&mut self) {
        if self.can_cancel_cut() {
            self.paths = Arc::default();
            self.cut = false;
            self.revision += 1;
        }
    }

    pub(super) fn pending_cut_paths(&self) -> &[PathBuf] {
        if self.cut { &self.paths } else { &[] }
    }

    fn finish_job(&mut self, outcome: &JobResult, revision: u64) {
        if matches!(outcome.kind, JobKind::Transfer { .. }) {
            self.pasting = false;
        }
        if self.revision != revision || !self.cut {
            return;
        }
        let sources = outcome
            .items
            .iter()
            .filter(|item| item.result.is_ok())
            .filter_map(|item| Some((item.source.as_deref()?, item.target.as_deref())));
        match outcome.kind {
            JobKind::Rename => {
                let mut sources = sources;
                if let Some((source, Some(target))) = sources.next()
                    && source != target
                {
                    self.paths = self
                        .paths
                        .iter()
                        .map(|path| match path.strip_prefix(source) {
                            Ok(suffix) if suffix.as_os_str().is_empty() => target.to_owned(),
                            Ok(suffix) => target.join(suffix),
                            Err(_) => path.clone(),
                        })
                        .collect();
                }
            }
            JobKind::Trash => {
                let sources = sources.map(|(source, _)| source).collect::<Vec<_>>();
                self.paths = self
                    .paths
                    .iter()
                    .filter(|path| !sources.iter().any(|source| path.starts_with(source)))
                    .cloned()
                    .collect();
            }
            JobKind::Transfer { cut: true } => {
                let sources = sources
                    .map(|(source, _)| source)
                    .collect::<std::collections::HashSet<_>>();
                self.paths = self
                    .paths
                    .iter()
                    .filter(|path| !sources.contains(path.as_path()))
                    .cloned()
                    .collect();
            }
            JobKind::Create | JobKind::Transfer { cut: false } => {}
        }
    }
}

#[derive(Default)]
pub struct FileJobs {
    running: usize,
    next_id: u64,
}

impl Global for FileJobs {}

impl FileJobs {
    pub fn quit_if_idle(cx: &mut gpui_kit::App) {
        if cx.windows().is_empty() && cx.global::<Self>().running == 0 {
            cx.quit();
        }
    }
}

enum NameTarget {
    Create { parent: PathBuf, folder: bool },
    Rename(PathBuf),
}

pub(super) struct FileNameEditor {
    target: NameTarget,
    input: Entity<InputState>,
    error: Option<String>,
    _enter: Subscription,
}

pub(super) struct DirectoryProperties {
    path: PathBuf,
    entries: usize,
    read_errors: usize,
    focus: FocusHandle,
}

#[derive(Clone)]
pub(super) struct FileMenuTarget {
    pub(super) pane_id: uuid::Uuid,
    pub(super) navigation_revision: u64,
    pub(super) path: Option<PathBuf>,
}

impl App {
    /// 键盘快捷键的分发入口（由 `keymap::SHORTCUTS` 的键绑定触发）。
    ///
    /// 焦点在输入框（重命名、搜索等）时不拦截也不执行，让按键继续
    /// 传给输入框做文本复制等操作。
    pub(super) fn file_shortcut(
        &mut self,
        action: FileAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.is_focused(window) || self.file_name_editor.is_some() {
            return;
        }
        cx.stop_propagation();
        self.file_action(action, None, window, cx);
    }

    pub(super) fn can_paste(&self, cx: &gpui_kit::App) -> bool {
        self.can_paste_into(None, cx)
    }

    pub(super) fn can_paste_into(&self, entry: Option<usize>, cx: &gpui_kit::App) -> bool {
        self.paste_directory(entry).is_some() && self.paste_available(cx)
    }

    fn paste_available(&self, cx: &gpui_kit::App) -> bool {
        let clipboard = cx.global::<FileClipboard>();
        !clipboard.paths.is_empty()
            && !clipboard.pasting
            && !self.file_job.is_some()
            && matches!(
                self.panes[self.pane_target].browser.state,
                DirectoryState::Ready(_)
            )
    }

    fn paste_directory(&self, entry: Option<usize>) -> Option<PathBuf> {
        match entry {
            Some(index) => {
                let (path, EntryKind::Directory) = self.entry_target(index)? else {
                    return None;
                };
                Some(path)
            }
            None => self.location().map(Path::to_owned),
        }
    }

    pub(super) fn file_action(
        &mut self,
        action: FileAction,
        menu_target: Option<&FileMenuTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_name_editor.is_some()
            || self.directory_properties.is_some()
            || (self.file_job.is_some()
                && !matches!(action, FileAction::SelectAll | FileAction::Properties))
        {
            return;
        }
        let mut menu_entry = None;
        if let Some(target) = menu_target {
            if target.pane_id != self.panes[self.pane_target].id
                || target.navigation_revision != self.panes[self.pane_target].navigation_revision
            {
                return;
            }
            if let Some(path) = &target.path {
                let Some(index) = self.panes[self.pane_target]
                    .browser
                    .visible_entry_index(path)
                else {
                    return;
                };
                menu_entry = Some(index);
                if self.panes[self.pane_target]
                    .browser
                    .select_for_context_menu(index)
                {
                    cx.notify();
                }
            }
        }
        window.focus(&self.focus, cx);
        match action {
            FileAction::SelectAll => {
                self.panes[self.pane_target].browser.select_all();
                cx.notify();
            }
            FileAction::Properties => self.open_directory_properties(window, cx),
            FileAction::NewFolder | FileAction::NewDocument => {
                if matches!(
                    self.panes[self.pane_target].browser.state,
                    DirectoryState::Ready(_)
                ) && let Some(path) = self.location()
                {
                    let folder = matches!(action, FileAction::NewFolder);
                    self.edit_file_name(
                        NameTarget::Create {
                            parent: path.to_owned(),
                            folder,
                        },
                        if folder {
                            "新建文件夹"
                        } else {
                            "新建文档.txt"
                        },
                        window,
                        cx,
                    );
                }
            }
            FileAction::Rename => {
                if self.panes[self.pane_target].browser.selection_len() != 1 {
                    return;
                }
                let entry = self.panes[self.pane_target]
                    .browser
                    .selected_entries()
                    .next();
                if let Some(entry) = entry.filter(|entry| entry.kind != EntryKind::Missing) {
                    let path = entry.path.clone();
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    self.edit_file_name(NameTarget::Rename(path), &name, window, cx);
                }
            }
            FileAction::Cut | FileAction::Copy | FileAction::Trash => {
                let paths: Vec<_> = self.panes[self.pane_target]
                    .browser
                    .selected_entries()
                    .filter(|entry| entry.kind != EntryKind::Missing)
                    .map(|entry| entry.path.clone())
                    .collect();
                if paths.is_empty() {
                    return;
                }
                if matches!(action, FileAction::Trash) {
                    self.run_file_job(FileJob::Trash(paths), window, cx);
                } else {
                    let clipboard = cx.global_mut::<FileClipboard>();
                    clipboard.revision += 1;
                    clipboard.paths = paths.into();
                    clipboard.cut = matches!(action, FileAction::Cut);
                    self.panes[self.pane_target].browser.action_error = None;
                    cx.notify();
                }
            }
            FileAction::Paste => {
                let entry = menu_entry.filter(|&index| {
                    self.entry_target(index)
                        .is_some_and(|(_, kind)| kind == EntryKind::Directory)
                });
                if self.paste_available(cx)
                    && let Some(directory) = self.paste_directory(entry)
                {
                    let clipboard = cx.global_mut::<FileClipboard>();
                    clipboard.pasting = true;
                    let job = FileJob::Transfer {
                        sources: clipboard.paths.clone(),
                        directory,
                        cut: clipboard.cut,
                    };
                    self.run_file_job(job, window, cx);
                }
            }
        }
    }

    fn edit_file_name(
        &mut self,
        target: NameTarget,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value.to_owned()));
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        let enter = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(
                event,
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false
                }
            ) {
                this.submit_file_name(window, cx);
            }
        });
        self.file_name_editor = Some(FileNameEditor {
            target,
            input,
            error: None,
            _enter: enter,
        });
        cx.notify();
    }

    fn close_file_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.file_name_editor = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn submit_file_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_job.is_some() {
            return;
        }
        let Some(editor) = &mut self.file_name_editor else {
            return;
        };
        let value = editor.input.read(cx).value();
        let target = match &editor.target {
            NameTarget::Create { parent, folder } => {
                files::named_path(parent, &value).map(|path| FileJob::Create {
                    path,
                    folder: *folder,
                })
            }
            NameTarget::Rename(source) => source
                .parent()
                .ok_or_else(|| io::Error::other("无法修改根目录名称"))
                .and_then(|parent| {
                    let original = source.file_name().unwrap_or_default();
                    if original.to_string_lossy() == value.as_str() {
                        Ok(source.clone())
                    } else {
                        files::named_path(parent, &value)
                    }
                })
                .map(|target| FileJob::Rename {
                    source: source.clone(),
                    target,
                }),
        };
        match target {
            Ok(job) => self.run_file_job(job, window, cx),
            Err(error) => {
                editor.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn run_file_job(&mut self, job: FileJob, window: &mut Window, cx: &mut Context<Self>) {
        let kind = job.kind();
        self.file_job = Some(kind);
        let jobs = cx.global_mut::<FileJobs>();
        jobs.running += 1;
        let job_id = jobs.next_id;
        jobs.next_id += 1;
        let clipboard_revision = cx.global::<FileClipboard>().revision;
        self.panes[self.pane_target].browser.action_error = None;
        if let Some(editor) = &mut self.file_name_editor {
            editor.error = None;
        }
        let pane_id = self.panes[self.pane_target].id;
        let navigation_revision = self.panes[self.pane_target].navigation_revision;
        let named_job = matches!(kind, JobKind::Create | JobKind::Rename);
        let work = cx.background_spawn(async move {
            tracing::debug!(job_id, ?kind, "开始文件任务");
            let started = std::time::Instant::now();
            let outcome = job.run();
            let mut errors = outcome
                .items
                .iter()
                .filter_map(|item| item.result.as_ref().err());
            if let Some(error) = errors.next() {
                let failures = 1 + errors.count();
                tracing::warn!(job_id, ?kind, failures, %error, "文件任务存在失败项");
            }
            tracing::debug!(
                job_id,
                ?kind,
                items = outcome.items.len(),
                elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
                "结束文件任务"
            );
            outcome
        });
        let app_context = cx.to_async();
        cx.spawn_in(window, async move |this, cx| {
            let outcome = work.await;
            // 窗口关闭后仍需结算任务并释放应用内剪贴板。
            app_context.update(|cx| {
                cx.global_mut::<FileJobs>().running -= 1;
                cx.global_mut::<FileClipboard>()
                    .finish_job(&outcome, clipboard_revision);
                FileJobs::quit_if_idle(cx);
            });
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_job = None;
                this.refresh_other_job_panes(pane_id, navigation_revision, &outcome, cx);
                let error_target = if this.panes.iter().any(|pane| pane.id == pane_id) {
                    pane_id
                } else {
                    this.panes[this.active_pane].id
                };
                this.with_pane(error_target, |this| {
                    let still_here = error_target == pane_id
                        && this.panes[this.pane_target].navigation_revision == navigation_revision;
                    let mut failures = outcome.items.iter().filter_map(|item| {
                        let error = item.result.as_ref().err()?;
                        Some((item, error))
                    });
                    if let Some((item, error)) = failures.next() {
                        let failure_count = 1 + failures.count();
                        let path = item.target.as_ref().or(item.source.as_ref());
                        let error = match path {
                            Some(path) if error.kind() == io::ErrorKind::AlreadyExists => {
                                format!("已存在同名项目：{}", path.display())
                            }
                            Some(path) => format!("{}：{error}", path.display()),
                            None => error.to_string(),
                        };
                        if named_job && let Some(editor) = &mut this.file_name_editor {
                            editor.error = Some(error);
                            editor.input.update(cx, |input, cx| input.focus(window, cx));
                        } else {
                            this.panes[this.pane_target].browser.action_error =
                                Some(format!("{} 项操作失败 · {error}", failure_count));
                            if still_here {
                                this.refresh_after_file_operation(None, cx);
                            }
                        }
                    } else {
                        if named_job && this.file_name_editor.is_some() {
                            this.close_file_name(window, cx);
                        }
                        let reveal = if named_job && still_here {
                            outcome
                                .items
                                .into_iter()
                                .next()
                                .and_then(|item| item.target)
                        } else {
                            None
                        };
                        if still_here {
                            this.refresh_after_file_operation(reveal, cx);
                        }
                    }
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_other_job_panes(
        &mut self,
        origin: uuid::Uuid,
        revision: u64,
        outcome: &JobResult,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<_> = outcome
            .items
            .iter()
            .flat_map(|item| {
                [item.source.as_deref(), item.target.as_deref()]
                    .into_iter()
                    .flatten()
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        for index in 0..self.panes.len() {
            let pane = &self.panes[index];
            if pane.id == origin && pane.navigation_revision == revision {
                continue;
            }
            self.with_pane(pane.id, |this| {
                use settings::config::tab::Location;

                let affected = match this.current_location() {
                    Some(Location::Directory { path }) => paths.iter().any(|changed| {
                        changed.parent() == Some(path.0.as_path()) || path.0.starts_with(changed)
                    }),
                    Some(Location::Tag { .. }) => true,
                    None => false,
                };
                if affected {
                    this.refresh_directory(cx);
                }
            });
        }
    }

    fn open_directory_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(path), DirectoryState::Ready(snapshot)) =
            (self.location(), &self.panes[self.pane_target].browser.state)
        else {
            return;
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.directory_properties = Some(DirectoryProperties {
            path: path.to_owned(),
            entries: snapshot.directory.entries.len(),
            read_errors: snapshot.directory.read_errors,
            focus,
        });
        cx.notify();
    }

    fn close_directory_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.directory_properties = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn directory_properties_dialog(&self, cx: &mut Context<Self>) -> Option<Dialog> {
        let properties = self.directory_properties.as_ref()?;
        let owner = cx.weak_entity();
        let name = properties
            .path
            .file_name()
            .unwrap_or(properties.path.as_os_str());
        Some(
            Dialog::new(cx)
                .focus_handle(properties.focus.clone())
                .on_open_change(move |open, _, window, cx| {
                    if !open {
                        let _ = owner.update(cx, |this, cx| {
                            this.close_directory_properties(window, cx);
                        });
                    }
                })
                .backdrop(div().size_full().bg(black().opacity(0.35)).occlude())
                .popup(
                    popup_surface("directory-properties", cx)
                        .occlude()
                        .w(px(440.))
                        .max_w(relative(0.9))
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("文件夹属性"))
                        .child(
                            v_flex()
                                .min_w_0()
                                .gap_1()
                                .text_sm()
                                .child(format!("名称　{}", name.to_string_lossy()))
                                .child("类型　文件夹")
                                .child(format!("路径　{}", properties.path.display()))
                                .child(format!("直接包含　{} 项", properties.entries))
                                .when(properties.read_errors > 0, |details| {
                                    details
                                        .child(format!("读取失败　{} 项", properties.read_errors))
                                }),
                        )
                        .child(
                            h_flex().justify_end().child(
                                Button::new("close-directory-properties")
                                    .ghost()
                                    .label("关闭")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_directory_properties(window, cx);
                                    })),
                            ),
                        ),
                ),
        )
    }

    pub(super) fn file_name_dialog(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let editor = self.file_name_editor.as_ref()?;
        let label = match editor.target {
            NameTarget::Create { folder: true, .. } => "新建文件夹",
            NameTarget::Create { folder: false, .. } => "新建文档",
            NameTarget::Rename(_) => "重命名",
        };
        let running = self.file_job.is_some();
        let danger = cx.theme().danger;
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x00000040))
                .occlude()
                .child(
                    popup_surface("file-name-dialog", cx)
                        .w(px(360.))
                        .p_5()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" && !this.file_job.is_some() {
                                cx.stop_propagation();
                                this.close_file_name(window, cx);
                            }
                        }))
                        .child(
                            v_flex()
                                .gap_3()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                                .child(Input::new(&editor.input).disabled(running))
                                .when_some(editor.error.clone(), |panel, error| {
                                    panel.child(div().text_sm().text_color(danger).child(error))
                                })
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("cancel-file-name")
                                                .label("取消")
                                                .disabled(running)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.close_file_name(window, cx)
                                                })),
                                        )
                                        .child(
                                            Button::new("save-file-name")
                                                .label(label)
                                                .primary()
                                                .disabled(running)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.submit_file_name(window, cx)
                                                })),
                                        ),
                                ),
                        ),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{FileClipboard, JobKind, JobResult, PathBuf};
    use daemon::job::ItemResult;

    #[test]
    fn cancel_cut_clears_only_pending_cut_paths() {
        let paths: std::sync::Arc<[PathBuf]> = [PathBuf::from("/source")].into();
        let mut state = FileClipboard {
            paths: paths.clone(),
            ..Default::default()
        };
        state.cancel_cut();
        assert_eq!(state.paths, paths);

        state.cut = true;
        state.pasting = true;
        state.cancel_cut();
        assert_eq!(state.paths, paths);

        state.pasting = false;
        state.cancel_cut();
        assert!(state.paths.is_empty());
        assert!(!state.cut);
        assert_eq!(state.revision, 1);
    }

    #[test]
    fn pending_cut_follows_successful_renames_and_removes_trashed_paths() {
        let mut state = FileClipboard {
            paths: ["/source", "/source/file", "/source-other"]
                .map(PathBuf::from)
                .into(),
            cut: true,
            ..Default::default()
        };
        state.finish_job(
            &JobResult {
                kind: JobKind::Rename,
                items: vec![ItemResult {
                    source: Some("/source".into()),
                    target: Some("/renamed".into()),
                    result: Ok(()),
                }],
            },
            0,
        );
        assert_eq!(
            state.pending_cut_paths(),
            ["/renamed", "/renamed/file", "/source-other"].map(PathBuf::from)
        );
        state.finish_job(
            &JobResult {
                kind: JobKind::Trash,
                items: vec![ItemResult {
                    source: Some("/renamed".into()),
                    target: None,
                    result: Ok(()),
                }],
            },
            0,
        );
        assert_eq!(state.pending_cut_paths(), [PathBuf::from("/source-other")]);
    }

    #[test]
    fn cut_paste_removes_only_successful_sources_from_the_same_clipboard() {
        let mut state = FileClipboard {
            paths: ["/moved", "/failed", "/other"].map(PathBuf::from).into(),
            cut: true,
            revision: 2,
            pasting: true,
        };
        let result = JobResult {
            kind: JobKind::Transfer { cut: true },
            items: vec![
                ItemResult {
                    source: Some("/moved".into()),
                    target: Some("/target/moved".into()),
                    result: Ok(()),
                },
                ItemResult {
                    source: Some("/failed".into()),
                    target: Some("/target/failed".into()),
                    result: Err(std::io::ErrorKind::AlreadyExists.into()),
                },
            ],
        };
        state.finish_job(&result, 1);
        assert_eq!(state.paths.len(), 3);
        state.finish_job(&result, 2);
        assert_eq!(
            state.pending_cut_paths(),
            ["/failed", "/other"].map(PathBuf::from)
        );
        assert!(!state.pasting);
    }
}
