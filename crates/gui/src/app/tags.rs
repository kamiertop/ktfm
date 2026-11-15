use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use hex_color::HexColor;
use settings::config::tab::Location;
use settings::tag::{Tag, TagItem};
use settings::tag_store::TagStore;
use uuid::Uuid;

use super::App;
use crate::components::icon_button;

pub(super) struct TagState {
    tags: Tag,
    store: Option<TagStore>,
    pub(super) load_error: Option<String>,
}

impl TagState {
    pub(super) fn open() -> Self {
        match TagStore::open() {
            Ok((store, tags)) => Self {
                tags,
                store: Some(store),
                load_error: None,
            },
            Err(error) => Self {
                tags: Tag::default(),
                store: None,
                load_error: Some(format!("无法打开标签数据库：{error}")),
            },
        }
    }

    pub(super) fn loaded_tags(&self) -> Option<&Tag> {
        self.store.as_ref().map(|_| &self.tags)
    }
}

pub(super) enum TagChange {
    Details,
    Paths(Uuid),
    Removed(Uuid),
}

impl EventEmitter<TagChange> for TagState {}

pub(super) struct TagEditor {
    id: Option<Uuid>,
    path: Option<PathBuf>,
    name: Entity<InputState>,
    color: Entity<InputState>,
    note: Entity<InputState>,
    error: Option<String>,
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
            path,
            name,
            color,
            note,
            error: None,
        });
        cx.notify();
    }

    fn apply_tag(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.tag_editor.as_mut() else {
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
        let path = editor.path.as_deref();
        // 先提交数据库，再更新所有窗口共用的标签数据。
        let result = self.tag_state.update(cx, |state, cx| {
            let store = state
                .store
                .as_mut()
                .ok_or_else(|| "标签数据库不可用".to_owned())?;
            let result = if let Some(id) = id {
                let existing = state
                    .tags
                    .get_mut(id)
                    .ok_or_else(|| "标签已不存在".to_owned())?;
                store
                    .update(id, &name, color, note.as_deref())
                    .map(|updated_at| existing.update(name, color, note, updated_at))
            } else {
                let mut tag = TagItem::new(name, color, note);
                if let Some(path) = path {
                    tag.set_path(path.to_owned(), true, tag.updated_at);
                }
                store.create(&tag).map(|()| state.tags.insert(tag))
            };
            result.map_err(|error| format!("保存标签失败：{error}"))?;
            cx.emit(TagChange::Details);
            Ok::<_, String>(())
        });
        match result {
            Ok(()) => {
                self.tag_editor = None;
                self.tag_error = None;
            }
            Err(error) => editor.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn toggle_tag_path(&mut self, id: Uuid, path: PathBuf, cx: &mut Context<Self>) {
        self.tag_error = self
            .tag_state
            .update(cx, |state, cx| {
                let store = state
                    .store
                    .as_mut()
                    .ok_or_else(|| "标签数据库不可用".to_owned())?;
                let tag = state
                    .tags
                    .get_mut(id)
                    .ok_or_else(|| "标签已不存在".to_owned())?;
                let present = !tag.contains(&path);
                let updated_at = store
                    .set_path(id, &path, present)
                    .map_err(|error| format!("更新标签关联失败：{error}"))?;
                tag.set_path(path, present, updated_at);
                cx.emit(TagChange::Paths(id));
                Ok::<_, String>(())
            })
            .err();
        cx.notify();
    }

    pub(super) fn delete_tag(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.tag_error = self
            .tag_state
            .update(cx, |state, cx| {
                let store = state
                    .store
                    .as_mut()
                    .ok_or_else(|| "标签数据库不可用".to_owned())?;
                store
                    .remove(id)
                    .map_err(|error| format!("删除标签失败：{error}"))?;
                state.tags.remove(id);
                cx.emit(TagChange::Removed(id));
                Ok::<_, String>(())
            })
            .err();
        cx.notify();
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
                    .child(div().flex_1().min_w_0().child(Input::new(&editor.name)))
                    .child(div().flex_1().min_w_0().child(Input::new(&editor.color)))
                    .child(div().flex_1().min_w_0().child(Input::new(&editor.note)))
                    .child(
                        icon_button("save-tag", IconName::Check, "保存标签")
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
