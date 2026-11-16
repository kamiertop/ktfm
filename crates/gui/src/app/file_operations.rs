use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use daemon::files;
use gpui_kit::base::{Dialog, Disableable as _};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use vfs::EntryKind;

use super::{App, browser::DirectoryState};
use crate::components::popup_surface;

#[derive(Clone, Copy)]
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

#[derive(Default)]
pub struct FileOperations {
    paths: Arc<[PathBuf]>,
    cut: bool,
    revision: u64,
    pasting: bool,
    running: usize,
}

impl Global for FileOperations {}

impl FileOperations {
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

enum FileJob {
    Create {
        path: PathBuf,
        folder: bool,
    },
    Rename(PathBuf, PathBuf),
    Paste {
        paths: Arc<[PathBuf]>,
        directory: PathBuf,
        cut: bool,
        revision: u64,
    },
    Trash(Vec<PathBuf>),
}

#[derive(Default)]
struct Outcome {
    reveal: Option<PathBuf>,
    failed: Vec<PathBuf>,
    first_error: Option<String>,
}

impl Outcome {
    fn record(&mut self, path: &Path, result: io::Result<()>) {
        if let Err(error) = result {
            self.first_error.get_or_insert_with(|| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    format!("已存在同名项目：{}", path.display())
                } else {
                    format!("{}：{error}", path.display())
                }
            });
            self.failed.push(path.to_owned());
        }
    }
}

impl FileJob {
    fn run(&self) -> Outcome {
        let mut result = Outcome::default();
        match self {
            Self::Create { path, folder } => {
                result.record(path, files::create(path, *folder));
                result.reveal = Some(path.clone());
            }
            Self::Rename(source, target) => {
                result.record(target, files::rename(source, target));
                result.reveal = Some(target.clone());
            }
            Self::Paste {
                paths,
                directory,
                cut,
                ..
            } => {
                for path in paths.iter() {
                    result.record(path, files::transfer(path, directory, *cut));
                }
            }
            Self::Trash(paths) => {
                for path in paths {
                    result.record(path, sys::trash::put(path));
                }
            }
        }
        result
    }
}

impl App {
    pub(super) fn handle_file_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.is_focused(window) || self.file_name_editor.is_some() {
            return;
        }
        let key = &event.keystroke;
        let modifiers = key.modifiers;
        if modifiers.alt || modifiers.platform || modifiers.shift {
            return;
        }
        let action = match (modifiers.control, key.key.as_str()) {
            (true, "c") => FileAction::Copy,
            (true, "x") => FileAction::Cut,
            (true, "v") => FileAction::Paste,
            (false, "f2") => FileAction::Rename,
            (false, "delete") => FileAction::Trash,
            (true, "a") => FileAction::SelectAll,
            _ => return,
        };
        cx.stop_propagation();
        if !event.is_held {
            self.file_action(action, None, window, cx);
        }
    }

    pub(super) fn can_paste(&self, cx: &gpui_kit::App) -> bool {
        let clipboard = cx.global::<FileOperations>();
        !clipboard.paths.is_empty()
            && !clipboard.pasting
            && !self.file_job_running
            && self.location().is_some()
            && matches!(self.browser.state, DirectoryState::Ready(_))
    }

    pub(super) fn file_action(
        &mut self,
        action: FileAction,
        menu_entry: Option<(u64, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_name_editor.is_some()
            || self.directory_properties.is_some()
            || (self.file_job_running
                && !matches!(action, FileAction::SelectAll | FileAction::Properties))
        {
            return;
        }
        if let Some((revision, index)) = menu_entry {
            if revision != self.browser.snapshot_revision {
                return;
            }
            if !self.browser.selection.contains(&index) {
                self.browser.clear_selection();
                self.browser.selection.insert(index);
                self.browser.anchor = Some(index);
                cx.notify();
            }
        }
        window.focus(&self.focus, cx);
        match action {
            FileAction::SelectAll => {
                self.browser.selection = self.browser.visible.iter().copied().collect();
                self.browser.anchor = self.browser.visible.first().copied();
                self.browser.reveal_path = None;
                cx.notify();
            }
            FileAction::Properties => self.open_directory_properties(window, cx),
            FileAction::NewFolder | FileAction::NewDocument => {
                if matches!(self.browser.state, DirectoryState::Ready(_))
                    && let Some(path) = self.location()
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
                if self.browser.selection.len() != 1 {
                    return;
                }
                if let Some((path, kind)) = self
                    .browser
                    .selection
                    .first()
                    .and_then(|&i| self.entry_target(i))
                    && kind != EntryKind::Missing
                {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    self.edit_file_name(NameTarget::Rename(path), &name, window, cx);
                }
            }
            FileAction::Cut | FileAction::Copy | FileAction::Trash => {
                let paths: Vec<_> = self
                    .browser
                    .selection
                    .iter()
                    .filter_map(|&index| self.entry_target(index))
                    .filter_map(|(path, kind)| (kind != EntryKind::Missing).then_some(path))
                    .collect();
                if paths.is_empty() {
                    return;
                }
                if matches!(action, FileAction::Trash) {
                    self.run_file_job(FileJob::Trash(paths), window, cx);
                } else {
                    let clipboard = cx.global_mut::<FileOperations>();
                    clipboard.revision += 1;
                    clipboard.paths = paths.into();
                    clipboard.cut = matches!(action, FileAction::Cut);
                    self.browser.action_error = None;
                    cx.notify();
                }
            }
            FileAction::Paste => {
                if self.can_paste(cx)
                    && let Some(directory) = self.location().map(Path::to_owned)
                {
                    let clipboard = cx.global_mut::<FileOperations>();
                    clipboard.pasting = true;
                    let job = FileJob::Paste {
                        paths: clipboard.paths.clone(),
                        directory,
                        cut: clipboard.cut,
                        revision: clipboard.revision,
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
        if self.file_job_running {
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
                .map(|target| FileJob::Rename(source.clone(), target)),
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
        self.file_job_running = true;
        cx.global_mut::<FileOperations>().running += 1;
        self.browser.action_error = None;
        if let Some(editor) = &mut self.file_name_editor {
            editor.error = None;
        }
        let navigation_revision = self.navigation_revision;
        let named_job = matches!(job, FileJob::Create { .. } | FileJob::Rename(..));
        let clipboard_job = match &job {
            FileJob::Paste { cut, revision, .. } => Some((*cut, *revision)),
            _ => None,
        };
        let work = cx.background_spawn(async move { job.run() });
        let app_context = cx.to_async();
        cx.spawn_in(window, async move |this, cx| {
            let outcome = work.await;
            let failure_count = outcome.failed.len();
            // 窗口关闭后仍需结算任务并释放应用内剪贴板。
            app_context.update(|cx| {
                let state = cx.global_mut::<FileOperations>();
                state.running -= 1;
                if let Some((cut, revision)) = clipboard_job {
                    state.pasting = false;
                    if cut && state.revision == revision {
                        state.paths = outcome.failed.into();
                    }
                }
                FileOperations::quit_if_idle(cx);
            });
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_job_running = false;
                let still_here = this.navigation_revision == navigation_revision;
                if let Some(error) = outcome.first_error {
                    if named_job && let Some(editor) = &mut this.file_name_editor {
                        editor.error = Some(error);
                        editor.input.update(cx, |input, cx| input.focus(window, cx));
                    } else {
                        this.browser.action_error =
                            Some(format!("{} 项操作失败 · {error}", failure_count));
                        this.refresh_after_file_operation(None, cx);
                    }
                } else {
                    if named_job && this.file_name_editor.is_some() {
                        this.close_file_name(window, cx);
                    }
                    this.refresh_after_file_operation(outcome.reveal.filter(|_| still_here), cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_directory_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(path), DirectoryState::Ready(snapshot)) = (self.location(), &self.browser.state)
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
        let running = self.file_job_running;
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
                            if event.keystroke.key == "escape" && !this.file_job_running {
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
