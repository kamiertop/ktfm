use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariant, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use hex_color::HexColor;
use settings::config::tab::Location;
use settings::tag::{Tag, TagItem};
use settings::tag_store::{OptionalField, TagPatch, TagStore};
use uuid::Uuid;

use super::App;
use super::io_queue::IoQueue;
use crate::components::{popup_surface, with_tooltip};

const TAG_COLORS: [(&str, u32); 9] = [
    ("红色", 0xc94246),
    ("橙色", 0xb65c24),
    ("金色", 0xa16d13),
    ("绿色", 0x38824b),
    ("青色", 0x227f76),
    ("蓝色", 0x376fb5),
    ("紫色", 0x7853b4),
    ("粉色", 0xb34e79),
    ("灰色", 0x647383),
];

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
    _enter: [Subscription; 3],
    _color_change: Subscription,
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
                let navigation = self.panes[self.pane_target].navigation_revision;
                if self.tag_state.read(cx).loaded_tags().is_some() {
                    self.forget_missing_tags(window, cx);
                }
                if self.panes[self.pane_target].navigation_revision == navigation
                    && matches!(self.current_location(), Some(Location::Tag { .. }))
                {
                    self.load_directory(cx);
                }
            }
            TagChange::Paths(id) => {
                if self.current_location() == Some(&Location::Tag { id }) {
                    self.refresh_directory(cx);
                }
            }
            TagChange::Removed(id) => {
                if self
                    .tag_editor
                    .as_ref()
                    .is_some_and(|editor| editor.id == Some(id))
                {
                    self.tag_editor = None;
                    window.focus(&self.focus, cx);
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
        let enter = [&name, &color, &note].map(|input| {
            cx.subscribe_in(input, window, |this, _, event, window, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: false
                    }
                ) {
                    this.apply_tag(window, cx);
                }
            })
        });
        let color_change = cx.subscribe(&color, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
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
            _enter: enter,
            _color_change: color_change,
        });
        cx.notify();
    }

    fn close_tag_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .tag_editor
            .as_ref()
            .is_some_and(|editor| !editor.saving)
        {
            self.tag_editor = None;
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn apply_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            self.close_tag_editor(window, cx);
            return;
        }
        let window = window.window_handle().downcast::<Self>();
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
                        if let Some(handle) = window {
                            cx.defer(move |cx| {
                                let _ = handle.update(cx, |this, window, cx| {
                                    if this.tag_editor.is_none() {
                                        window.focus(&this.focus, cx);
                                    }
                                });
                            });
                        }
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

    fn set_tag_editor_color(&mut self, color: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.tag_editor.as_ref().filter(|editor| !editor.saving) else {
            return;
        };
        editor
            .color
            .update(cx, |input, cx| input.set_value(color, window, cx));
        cx.notify();
    }

    fn tag_color_palette(
        &self,
        editor: &TagEditor,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let value = editor.color.read(cx).value();
        let selected = HexColor::parse_rgb(value.trim()).ok();
        let theme = cx.theme();
        let (border, ring, surface) = (theme.border, theme.foreground, theme.muted.opacity(0.35));
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(div().text_sm().child("颜色"))
                    .child(
                        Button::new("no-tag-color")
                            .ghost()
                            .small()
                            .label("无颜色")
                            .disabled(editor.saving || value.trim().is_empty())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_tag_editor_color(String::new(), window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .p_1()
                    .flex_wrap()
                    .rounded_lg()
                    .border_1()
                    .border_color(border)
                    .bg(surface)
                    .children(TAG_COLORS.map(|(name, color)| {
                        let hex = format!("#{color:06X}");
                        let active = selected.is_some_and(|selected| {
                            u32::from_be_bytes([0, selected.r, selected.g, selected.b]) == color
                        });
                        let swatch = Button::new(("tag-color", color as u64))
                            .with_variant(ButtonVariant::Custom(ButtonCustomVariant::new(cx)))
                            .w(px(30.))
                            .h(px(30.))
                            .p_0()
                            .rounded_full()
                            .accessibility_label(name)
                            .toggled(active)
                            .disabled(editor.saving)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(22.))
                                    .rounded_full()
                                    .bg(rgb(color))
                                    .text_color(white())
                                    .when(active, |dot| {
                                        dot.child(Icon::new(IconName::Check).size(px(13.)))
                                    }),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_tag_editor_color(hex.clone(), window, cx);
                            }));
                        div()
                            .id(("tag-color-ring", color as u64))
                            .rounded_full()
                            .border_1()
                            .border_color(if active { ring } else { transparent_black() })
                            .when(!active && !editor.saving, |outline| {
                                outline.hover(|style| style.border_color(ring.opacity(0.5)))
                            })
                            .child(with_tooltip(swatch, name))
                    })),
            )
            .child(Input::new(&editor.color).disabled(editor.saving))
    }

    pub(super) fn tag_editor_dialog(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let editor = self.tag_editor.as_ref()?;
        let danger = cx.theme().danger;
        let title = if editor.id.is_some() {
            "编辑标签"
        } else {
            "新建标签"
        };
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
                    popup_surface("tag-editor-dialog", cx)
                        .w(px(400.))
                        .max_w(relative(0.9))
                        .p_5()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                this.close_tag_editor(window, cx);
                            }
                        }))
                        .child(
                            v_flex()
                                .gap_3()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(div().text_sm().child("名称"))
                                        .child(Input::new(&editor.name).disabled(editor.saving)),
                                )
                                .child(self.tag_color_palette(editor, cx))
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(div().text_sm().child("备注"))
                                        .child(Input::new(&editor.note).disabled(editor.saving)),
                                )
                                .when_some(editor.error.clone(), |form, error| {
                                    form.child(div().text_sm().text_color(danger).child(error))
                                })
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("cancel-tag")
                                                .ghost()
                                                .label("取消")
                                                .disabled(editor.saving)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.close_tag_editor(window, cx)
                                                })),
                                        )
                                        .child(
                                            Button::new("save-tag")
                                                .primary()
                                                .label(if editor.saving {
                                                    "正在保存…"
                                                } else {
                                                    "保存"
                                                })
                                                .disabled(editor.saving)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.apply_tag(window, cx)
                                                })),
                                        ),
                                ),
                        ),
                ),
        )
    }
}
