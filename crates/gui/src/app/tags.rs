use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use hex_color::HexColor;
use settings::config::tab::Location;
use settings::tag::{Tag, TagItem};
use settings::tag_store::{OptionalField, TagPatch, TagStore};
use uuid::Uuid;

use super::App;
use super::io_queue::IoQueue;
use crate::components::icon_button;

pub(super) struct TagState {
    tags: Tag,
    database: TagDatabase,
}

enum TagDatabase {
    Loading,
    Ready(TagStore),
    Failed { message: String, retryable: bool },
}

impl TagState {
    pub(super) fn loading() -> Self {
        Self {
            tags: Tag::default(),
            database: TagDatabase::Loading,
        }
    }

    fn open() -> Self {
        match TagStore::open() {
            Ok((store, tags)) => Self {
                tags,
                database: TagDatabase::Ready(store),
            },
            Err(error) => {
                let retryable = matches!(&error, settings::Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut);
                Self {
                    tags: Tag::default(),
                    database: TagDatabase::Failed {
                        message: format!("无法打开标签数据库：{error}"),
                        retryable,
                    },
                }
            }
        }
    }

    fn store(&self) -> Option<&TagStore> {
        match &self.database {
            TagDatabase::Ready(store) => Some(store),
            TagDatabase::Loading | TagDatabase::Failed { .. } => None,
        }
    }

    pub(super) fn load_error(&self) -> Option<&str> {
        match &self.database {
            TagDatabase::Loading | TagDatabase::Ready(_) => None,
            TagDatabase::Failed { message, .. } => Some(message),
        }
    }

    pub(super) fn loaded_tags(&self) -> Option<&Tag> {
        self.store().map(|_| &self.tags)
    }

    pub(super) fn load_in_background(state: &Entity<Self>, cx: &mut gpui_kit::App) {
        let state = state.downgrade();
        cx.spawn(async move |cx| {
            loop {
                let fresh = cx.background_spawn(async { Self::open() }).await;
                let retry = matches!(
                    fresh.database,
                    TagDatabase::Failed {
                        retryable: true,
                        ..
                    }
                );
                if cx
                    .update(|cx| {
                        state.update(cx, |state, cx| {
                            *state = fresh;
                            cx.emit(TagChange::Loaded);
                        })
                    })
                    .is_err()
                    || !retry
                {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
            }
        })
        .detach();
    }
}

pub(super) enum TagChange {
    Loaded,
    Details,
    Paths(Uuid),
    Removed(Uuid),
}

impl EventEmitter<TagChange> for TagState {}

enum TagMutation {
    Saved(TagItem, TagChange),
    Removed(Uuid),
}

pub(super) struct TagEditor {
    id: Option<Uuid>,
    original: (String, String, String),
    saving: bool,
    path: Option<PathBuf>,
    name: Entity<InputState>,
    color: Entity<InputState>,
    note: Entity<InputState>,
    error: Option<String>,
}

fn optional_change<T>(changed: bool, value: Option<T>) -> OptionalField<T> {
    if changed {
        value.map_or(OptionalField::Clear, OptionalField::Set)
    } else {
        OptionalField::Keep
    }
}

pub(super) fn tag_color(color: HexColor) -> Hsla {
    rgb(u32::from_be_bytes([0, color.r, color.g, color.b])).into()
}

impl App {
    pub(super) fn tags<'a>(&self, cx: &'a gpui_kit::App) -> &'a Tag {
        &self.tag_state.read(cx).tags
    }

    pub(super) fn sync_tag_change(
        &mut self,
        change: &TagChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match *change {
            TagChange::Details => {}
            TagChange::Loaded => {
                let navigation = self.navigation_revision;
                if self.tag_state.read(cx).loaded_tags().is_some() {
                    self.forget_missing_tags(window, cx);
                }
                if self.navigation_revision == navigation
                    && matches!(self.current_location(), Some(Location::Tag { .. }))
                {
                    self.load_directory(cx);
                }
            }
            TagChange::Paths(id) => {
                if self.current_location() == Some(&Location::Tag { id }) {
                    self.load_directory(cx);
                }
            }
            TagChange::Removed(id) => {
                if self
                    .tag_editor
                    .as_ref()
                    .is_some_and(|editor| editor.id == Some(id))
                {
                    self.tag_editor = None;
                }
                self.forget_tag(id, window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn edit_tag(
        &mut self,
        id: Option<Uuid>,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tag = id.and_then(|id| self.tags(cx).get(id));
        let name = tag.map_or_else(String::new, |tag| tag.name.clone());
        let color = tag
            .and_then(|tag| tag.color)
            .map_or_else(String::new, |color| color.display_rgb().to_string());
        let note = tag.and_then(|tag| tag.note.clone()).unwrap_or_default();
        let original = (name.clone(), color.clone(), note.clone());
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("名称")
                .default_value(name)
        });
        let color = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("颜色 #RRGGBB（可选）")
                .default_value(color)
        });
        let note = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("备注（可选）")
                .default_value(note)
        });
        name.update(cx, |input, cx| input.focus(window, cx));
        self.tag_editor = Some(TagEditor {
            id,
            original,
            saving: false,
            path,
            name,
            color,
            note,
            error: None,
        });
        cx.notify();
    }

    fn apply_tag(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.tag_editor.as_mut().filter(|editor| !editor.saving) else {
            return;
        };
        let name = editor.name.read(cx).value().trim().to_owned();
        let color = editor.color.read(cx).value().trim().to_owned();
        let note = editor.note.read(cx).value().trim().to_owned();
        if name.is_empty() {
            editor.error = Some("请输入标签名称".into());
            cx.notify();
            return;
        }
        let (original_name, original_color, original_note) = &editor.original;
        let name_changed = name != original_name.trim();
        let color_changed = !color.eq_ignore_ascii_case(original_color.trim());
        let note_changed = note != original_note.trim();
        let color = if color.is_empty() {
            None
        } else if let Ok(color) = HexColor::parse_rgb(&color) {
            Some(color)
        } else {
            editor.error = Some("颜色格式应为 #RRGGBB".into());
            cx.notify();
            return;
        };
        let note = (!note.is_empty()).then_some(note);
        let id = editor.id;
        if id.is_some() && !name_changed && !color_changed && !note_changed {
            self.tag_editor = None;
            cx.notify();
            return;
        }
        let path = editor.path.clone();
        let editor_name = editor.name.clone();
        editor.saving = true;
        self.run_tag_operation(
            move |store| {
                let tag = if let Some(id) = id {
                    store.update(
                        id,
                        TagPatch {
                            name: name_changed.then_some(name.as_str()),
                            color: optional_change(color_changed, color),
                            note: optional_change(note_changed, note.as_deref()),
                        },
                    )?
                } else {
                    let mut tag = TagItem::new(name, color, note);
                    if let Some(path) = path {
                        tag.set_path(path, true, tag.updated_at);
                    }
                    store.create(&tag)?;
                    tag
                };
                Ok(TagMutation::Saved(tag, TagChange::Details))
            },
            move |this, error, cx| {
                if let Some(editor) = this
                    .tag_editor
                    .as_mut()
                    .filter(|editor| editor.name == editor_name)
                {
                    if let Some(error) = error {
                        editor.error = Some(format!("保存标签失败：{error}"));
                        editor.saving = false;
                    } else {
                        this.tag_editor = None;
                        this.tag_error = None;
                    }
                }
                cx.notify();
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn set_tag_path(
        &mut self,
        id: Uuid,
        path: PathBuf,
        present: bool,
        cx: &mut Context<Self>,
    ) {
        self.run_tag_operation(
            move |store| {
                let tag = store.set_path(id, &path, present)?;
                Ok(TagMutation::Saved(tag, TagChange::Paths(id)))
            },
            |this, error, cx| {
                this.tag_error = error.map(|error| format!("更新标签关联失败：{error}"));
                cx.notify();
            },
            cx,
        );
    }

    pub(super) fn delete_tag(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.run_tag_operation(
            move |store| {
                store.remove(id)?;
                Ok(TagMutation::Removed(id))
            },
            |this, error, cx| {
                this.tag_error = error.map(|error| format!("删除标签失败：{error}"));
                cx.notify();
            },
            cx,
        );
    }

    fn run_tag_operation(
        &self,
        work: impl FnOnce(TagStore) -> settings::Result<TagMutation> + Send + 'static,
        complete: impl FnOnce(&mut Self, Option<String>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.tag_state.read(cx).store().cloned() else {
            let owner = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |this, cx| {
                    complete(this, Some("标签数据库不可用".to_owned()), cx);
                });
            });
            return;
        };
        let state = self.tag_state.clone();
        let owner = cx.weak_entity();
        IoQueue::submit(
            cx,
            move || work(store),
            move |result, cx| {
                let error = match result {
                    Ok(mutation) => {
                        state.update(cx, |state, cx| match mutation {
                            TagMutation::Saved(tag, mut change) => {
                                if let Some(existing) = state.tags.get_mut(tag.id) {
                                    if existing.paths != tag.paths {
                                        change = TagChange::Paths(tag.id);
                                    }
                                    *existing = tag;
                                } else {
                                    state.tags.insert(tag);
                                }
                                cx.emit(change);
                            }
                            TagMutation::Removed(id) => {
                                state.tags.remove(id);
                                cx.emit(TagChange::Removed(id));
                            }
                        });
                        None
                    }
                    Err(error) => Some(error.to_string()),
                };
                let _ = owner.update(cx, |this, cx| complete(this, error, cx));
            },
        );
    }

    pub(super) fn tag_form(
        &self,
        editor: &TagEditor,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (border, danger) = (theme.border, theme.danger);
        v_flex()
            .flex_shrink_0()
            .p_2()
            .gap_1()
            .border_b_1()
            .border_color(border)
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&editor.name).disabled(editor.saving)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&editor.color).disabled(editor.saving)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&editor.note).disabled(editor.saving)),
                    )
                    .child(
                        icon_button("save-tag", IconName::Check, "保存标签")
                            .disabled(editor.saving)
                            .on_click(cx.listener(|this, _, _, cx| this.apply_tag(cx))),
                    )
                    .child(
                        icon_button("cancel-tag", IconName::X, "取消").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.tag_editor = None;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .when_some(editor.error.clone(), |form, error| {
                form.child(div().text_sm().text_color(danger).child(error))
            })
    }
}
