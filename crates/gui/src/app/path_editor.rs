use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::App;
use super::keymap::{CancelPathEdit, NextPathSuggestion, PreviousPathSuggestion, SubmitPathEdit};
use crate::components::popup_surface;
use crate::style::file_selection_color;

const MAX_SUGGESTIONS: usize = 6;

pub(super) struct PathEditor {
    input: Entity<InputState>,
    suggestions: Vec<String>,
    selected: Option<usize>,
    error: Option<String>,
    _input_events: Subscription,
    _blur: Subscription,
    work: Option<Task<()>>,
}

impl App {
    pub(super) fn toggle_path_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane_id = self.pane().id;
        if self.pane().path_editor.is_some() {
            self.close_path_editor(window, cx);
            return;
        }
        let value = self.location().and_then(Path::to_str).unwrap_or("");
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入路径")
                .default_value(value.to_owned())
        });
        let input_events = cx.subscribe_in(&input, window, move |this, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.with_pane(pane_id, |this| this.check_path_input(false, window, cx));
            }
        });
        let blur = cx.on_blur(
            &input.read(cx).focus_handle(cx),
            window,
            move |this, _, cx| {
                this.with_pane(pane_id, |this| {
                    this.panes[this.pane_target].path_editor = None;
                    cx.notify();
                });
            },
        );
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.panes[self.pane_target].path_editor = Some(PathEditor {
            input,
            suggestions: Vec::new(),
            selected: None,
            error: None,
            _input_events: input_events,
            _blur: blur,
            work: None,
        });
        cx.notify();
    }

    pub(super) fn close_path_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.panes[self.pane_target].path_editor.take() {
            if editor.input.read(cx).focus_handle(cx).is_focused(window) {
                window.focus(&self.focus, cx);
            }
            cx.notify();
        }
    }

    fn check_path_input(&mut self, submit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.pane();
        let pane_id = pane.id;
        let show_hidden = pane.browser.show_hidden;
        let home = self.home().map(Path::to_owned);
        let base = self.location().map(Path::to_owned).or_else(|| home.clone());
        let Some(editor) = &mut self.panes[self.pane_target].path_editor else {
            return;
        };
        let value = editor.input.read(cx).value();
        editor.work = None;
        editor.error = None;
        editor.suggestions.clear();
        editor.selected = None;
        cx.notify();

        if value.is_empty() {
            if submit {
                editor.error = Some("请输入路径".into());
            }
            return;
        }

        editor.work = Some(cx.spawn_in(window, async move |this, cx| {
            let executor = cx.background_executor();
            if !submit {
                executor.timer(Duration::from_millis(100)).await;
            }
            let result = executor
                .spawn(async move {
                    if submit {
                        PathLookup::Open(validate_location(
                            &value,
                            base.as_deref(),
                            home.as_deref(),
                        ))
                    } else {
                        PathLookup::Suggest(path_suggestions(
                            &value,
                            base.as_deref(),
                            home.as_deref(),
                            show_hidden,
                        ))
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.with_pane(pane_id, |this| {
                    let Some(editor) = &mut this.panes[this.pane_target].path_editor else {
                        return;
                    };
                    match result {
                        PathLookup::Open(Ok((path, kind))) => {
                            this.close_path_editor(window, cx);
                            if kind == vfs::EntryKind::Directory {
                                this.navigate(path, window, cx);
                            } else {
                                this.open_file_target(path, kind, window, cx);
                            }
                        }
                        PathLookup::Open(Err(error)) => editor.error = Some(error),
                        PathLookup::Suggest(suggestions) => editor.suggestions = suggestions,
                    }
                    cx.notify();
                });
            });
        }));
    }

    fn move_path_suggestion(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.panes[self.pane_target].path_editor.as_mut() else {
            cx.propagate();
            return;
        };
        let count = editor.suggestions.len();
        if count == 0 {
            cx.propagate();
            return;
        }
        editor.selected = Some(match (editor.selected, forward) {
            (Some(index), true) => (index + 1) % count,
            (Some(index), false) => (index + count - 1) % count,
            (None, true) => 0,
            (None, false) => count - 1,
        });
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn path_editor_view(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let editor = self.pane().path_editor.as_ref()?;
        let input = &editor.input;
        let danger = cx.theme().danger;
        let hover = cx.theme().list_hover;
        let selected_bg = file_selection_color(cx.theme().mode);
        let selected_fg = cx.theme().popover_foreground;
        let muted = cx.theme().muted_foreground;
        Some(
            v_flex()
                .id("path-editor")
                .key_context("PathEditor")
                .on_action(cx.listener(|this, _: &CancelPathEdit, window, cx| {
                    cx.stop_propagation();
                    this.close_path_editor(window, cx);
                }))
                .on_action(cx.listener(|this, _: &SubmitPathEdit, window, cx| {
                    cx.stop_propagation();
                    if let Some(editor) = &this.pane().path_editor
                        && let Some(value) = editor
                            .selected
                            .and_then(|index| editor.suggestions.get(index))
                    {
                        fill_path_suggestion(&editor.input, value, window, cx);
                        return;
                    }
                    this.check_path_input(true, window, cx);
                }))
                .on_action(cx.listener(|this, _: &NextPathSuggestion, _, cx| {
                    this.move_path_suggestion(true, cx);
                }))
                .on_action(cx.listener(|this, _: &PreviousPathSuggestion, _, cx| {
                    this.move_path_suggestion(false, cx);
                }))
                .min_w_0()
                .flex_1()
                .relative()
                .child(Input::new(input).small())
                .when_some(editor.error.clone(), |view, error| {
                    view.child(
                        div()
                            .id("path-editor-error")
                            .truncate()
                            .text_xs()
                            .text_color(danger)
                            .child(error.clone())
                            .tooltip(move |window, cx| {
                                Tooltip::new(error.clone()).build(window, cx)
                            }),
                    )
                })
                .when(!editor.suggestions.is_empty(), |view| {
                    view.child(deferred(
                        popup_surface("path-completions", cx)
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(relative(1.))
                            .occlude()
                            .p_1()
                            .children(editor.suggestions.iter().enumerate().map(
                                |(index, suggestion)| {
                                    let input = input.clone();
                                    let value = suggestion.clone();
                                    let end = suggestion
                                        .trim_end_matches(MAIN_SEPARATOR)
                                        .rfind(MAIN_SEPARATOR)
                                        .map_or(0, |index| index + MAIN_SEPARATOR.len_utf8());
                                    let (parent, name) = suggestion.split_at(end);
                                    let selected = editor.selected == Some(index);
                                    h_flex()
                                        .id(("path-suggestion", index))
                                        .h_6()
                                        .min_w_0()
                                        .px_1()
                                        .text_sm()
                                        .rounded_sm()
                                        .role(Role::Button)
                                        .when(selected, |row| row.bg(selected_bg))
                                        .cursor_pointer()
                                        .when(!selected, |row| {
                                            row.hover(move |style| style.bg(hover))
                                        })
                                        .on_click(move |_, window, cx| {
                                            fill_path_suggestion(&input, &value, window, cx)
                                        })
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(if selected {
                                                    selected_fg
                                                } else {
                                                    muted
                                                })
                                                .child(parent.to_owned()),
                                        )
                                        .child(div().min_w_0().truncate().child(name.to_owned()))
                                        .tooltip({
                                            let suggestion = suggestion.clone();
                                            move |window, cx| {
                                                Tooltip::new(suggestion.clone()).build(window, cx)
                                            }
                                        })
                                },
                            )),
                    ))
                }),
        )
    }
}

fn fill_path_suggestion(
    input: &Entity<InputState>,
    value: &str,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    input.update(cx, |input, cx| {
        input.replace_all(value.to_owned(), window, cx);
        input.focus(window, cx);
    });
}

enum PathLookup {
    Open(Result<(PathBuf, vfs::EntryKind), String>),
    Suggest(Vec<String>),
}

fn resolve_path(value: &str, base: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        Some(path.to_owned())
    } else if value == "~" {
        home.map(Path::to_owned)
    } else if let Some(rest) = value.strip_prefix("~/") {
        home.map(|home| home.join(rest.trim_start_matches('/')))
    } else {
        base.map(|base| base.join(path))
    }
}

fn validate_location(
    value: &str,
    base: Option<&Path>,
    home: Option<&Path>,
) -> Result<(PathBuf, vfs::EntryKind), String> {
    let path = resolve_path(value, base, home).ok_or("无法解析此路径，请输入绝对路径")?;
    let path = path.canonicalize().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => "找不到该目录".into(),
        std::io::ErrorKind::PermissionDenied => "没有权限访问该路径".into(),
        _ => format!("无法打开目录：{error}"),
    })?;
    let metadata = path
        .metadata()
        .map_err(|error| format!("无法访问路径：{error}"))?;
    let kind = if metadata.is_dir() {
        vfs::EntryKind::Directory
    } else if metadata.is_file() {
        vfs::EntryKind::File
    } else {
        return Err("此路径不是普通文件或目录".into());
    };
    Ok((path, kind))
}

fn path_suggestions(
    value: &str,
    base: Option<&Path>,
    home: Option<&Path>,
    show_hidden: bool,
) -> Vec<String> {
    let Some(path) = resolve_path(value, base, home) else {
        return Vec::new();
    };
    let (entries, prefix, partial) = match path.read_dir() {
        Ok(entries) => {
            let mut prefix = value.to_owned();
            if !prefix.ends_with(MAIN_SEPARATOR) {
                prefix.push(MAIN_SEPARATOR);
            }
            (entries, prefix, "")
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) && !value.ends_with(MAIN_SEPARATOR) =>
        {
            let start = value
                .rfind(MAIN_SEPARATOR)
                .map_or(0, |index| index + MAIN_SEPARATOR.len_utf8());
            let (parent, partial) = value.split_at(start);
            let Some(entries) =
                resolve_path(parent, base, home).and_then(|path| path.read_dir().ok())
            else {
                return Vec::new();
            };
            (entries, parent.to_owned(), partial)
        }
        Err(_) => return Vec::new(),
    };
    let mut suggestions = std::collections::BTreeSet::new();
    for entry in entries.flatten() {
        let Ok(entry_name) = entry.file_name().into_string() else {
            continue;
        };
        if !entry_name.starts_with(partial)
            || entry_name == partial
            || (!show_hidden && !partial.starts_with('.') && entry_name.starts_with('.'))
        {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let directory = kind.is_dir() || (kind.is_symlink() && entry.path().is_dir());
        if directory {
            suggestions.insert(format!("{prefix}{entry_name}{MAIN_SEPARATOR}"));
        } else if kind.is_file() || (kind.is_symlink() && entry.path().is_file()) {
            suggestions.insert(format!("{prefix}{entry_name}"));
        }
        if suggestions.len() > MAX_SUGGESTIONS {
            suggestions.pop_last();
        }
    }
    suggestions.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{Path, path_suggestions, resolve_path, validate_location};

    #[test]
    fn expands_home_and_resolves_relative_paths_against_the_current_directory() {
        let home = Path::new("/home/test");
        let base = Path::new("/work");
        for (input, expected) in [
            ("~", "/home/test"),
            ("~//my files", "/home/test/my files"),
            ("my files", "/work/my files"),
            ("/tmp/my files", "/tmp/my files"),
        ] {
            assert_eq!(
                resolve_path(input, Some(base), Some(home)).as_deref(),
                Some(Path::new(expected)),
            );
        }
        assert_eq!(resolve_path("relative", None, None), None);
    }

    #[test]
    fn suggests_matching_entries_and_validates_the_destination() -> std::io::Result<()> {
        let root = std::env::temp_dir().join(format!("ktfm-test-path-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        let result = (|| {
            std::fs::create_dir_all(root.join("my folder").join("child"))?;
            std::fs::create_dir(root.join(".hidden folder"))?;
            std::fs::write(root.join("my file"), "")?;
            assert_eq!(
                path_suggestions("~/", Some(&root), Some(&root), false),
                ["~/my file", "~/my folder/"]
            );
            assert_eq!(
                path_suggestions("./", Some(&root), None, false),
                ["./my file", "./my folder/"]
            );
            assert_eq!(
                path_suggestions("./", Some(&root), None, true),
                ["./.hidden folder/", "./my file", "./my folder/"]
            );
            for input in ["./my folder", "./my folder/"] {
                assert_eq!(
                    path_suggestions(input, Some(&root), None, false),
                    ["./my folder/child/"],
                );
            }
            assert_eq!(
                path_suggestions("./my fold", Some(&root), None, false),
                ["./my folder/"]
            );
            assert!(path_suggestions("./my file", Some(&root), None, false).is_empty());
            assert_eq!(
                validate_location("my folder/..", Some(&root), None),
                Ok((root.canonicalize()?, vfs::EntryKind::Directory))
            );
            assert_eq!(
                validate_location("my file", Some(&root), None).unwrap().1,
                vfs::EntryKind::File
            );
            assert!(validate_location("missing", Some(&root), None).is_err());
            Ok(())
        })();
        std::fs::remove_dir_all(root)?;
        result
    }

    #[test]
    fn completes_only_the_last_component_of_a_path() -> std::io::Result<()> {
        let root = std::env::temp_dir().join(format!("ktfm-test-path-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        let result = (|| {
            std::fs::create_dir_all(root.join("a/b/cxx-work"))?;
            std::fs::create_dir_all(root.join("a/b/.cxx-cache"))?;
            std::fs::create_dir(root.join("a/b/notes"))?;
            std::fs::write(root.join("a/b/cxx.txt"), "")?;

            for prefix in [
                "a/b/".into(),
                "~/a/b/".into(),
                format!("{}/a/b/", root.display()),
            ] {
                assert_eq!(
                    path_suggestions(&format!("{prefix}cxx"), Some(&root), Some(&root), false),
                    [format!("{prefix}cxx-work/"), format!("{prefix}cxx.txt")]
                );
            }
            assert_eq!(
                path_suggestions("a/b/.cx", Some(&root), None, false),
                ["a/b/.cxx-cache/"]
            );
            assert!(path_suggestions("a/b/cxx/", Some(&root), None, false).is_empty());
            assert!(path_suggestions("a/missing/cxx", Some(&root), None, false).is_empty());
            Ok(())
        })();
        std::fs::remove_dir_all(root)?;
        result
    }
}
