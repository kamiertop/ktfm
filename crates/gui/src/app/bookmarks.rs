use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Dialog, Disableable as _};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::bookmark::{Bookmark, BookmarkKind, Bookmarks};
use uuid::Uuid;
use vfs::EntryKind;

use super::App;
use super::io_queue::IoQueue;
use super::workspace::WorkspaceWriter;
use crate::components::popup_surface;
use crate::style::{SIDEBAR_ICON_SIZE, SIDEBAR_WIDTH, icon_color};

pub(super) enum BookmarkDrag {
    Folder(PathBuf),
    Existing {
        id: Uuid,
        name: String,
        icon: IconName,
        color: Option<Hsla>,
    },
}

impl BookmarkDrag {
    pub(super) fn preview(
        &self,
        cursor_offset: Point<Pixels>,
        _: &mut Window,
        cx: &mut gpui_kit::App,
    ) -> Entity<BookmarkDragPreview> {
        let (name, icon, color) = match self {
            Self::Folder(path) => (
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
                IconName::Folder,
                None,
            ),
            Self::Existing {
                name, icon, color, ..
            } => (name.clone(), *icon, *color),
        };
        cx.new(|_| BookmarkDragPreview {
            name,
            icon,
            color,
            cursor_offset,
        })
    }
}

pub(super) struct BookmarkDragPreview {
    name: String,
    icon: IconName,
    color: Option<Hsla>,
    cursor_offset: Point<Pixels>,
}

impl Render for BookmarkDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // GPUI 从鼠标位置减去起拖偏移来定位预览，补回偏移后只留固定间距。
        let offset = self.cursor_offset + point(px(12.), px(12.));
        let color = self
            .color
            .unwrap_or_else(|| icon_color(self.icon, cx.theme().mode));
        div().child(
            popup_surface("bookmark-folder-drag", cx)
                .relative()
                .left(offset.x)
                .top(offset.y)
                .max_w(px(220.))
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Icon::new(self.icon)
                        .size(SIDEBAR_ICON_SIZE)
                        .flex_shrink_0()
                        .text_color(color),
                )
                .child(div().min_w_0().truncate().child(self.name.clone())),
        )
    }
}

pub(super) struct BookmarkProperties {
    id: Uuid,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
}

pub(super) struct BookmarkRename {
    id: Uuid,
    input: Entity<InputState>,
    position: Point<Pixels>,
    previous_focus: Option<FocusHandle>,
    _submit_subscription: Subscription,
    saving: bool,
}

impl App {
    pub(super) fn drop_bookmark(
        &mut self,
        drag: &BookmarkDrag,
        before: Option<Uuid>,
        cx: &mut Context<Self>,
    ) {
        let bookmarks = self.bookmarks.read(cx);
        if before.is_some_and(|id| bookmarks.get(id).is_none()) {
            return;
        }
        match drag {
            BookmarkDrag::Folder(path) => {
                if bookmarks.find(path).is_some() {
                    return;
                }
                let path = path.clone();
                self.change_bookmarks(
                    move |bookmarks| {
                        if bookmarks.find(&path).is_none()
                            && before.is_none_or(|id| bookmarks.get(id).is_some())
                        {
                            let id = bookmarks.add(path, BookmarkKind::Directory);
                            bookmarks.move_before(id, before);
                        }
                    },
                    cx,
                );
            }
            BookmarkDrag::Existing { id, .. }
                if Some(*id) != before && bookmarks.get(*id).is_some() =>
            {
                let id = *id;
                self.change_bookmarks(move |bookmarks| bookmarks.move_before(id, before), cx);
            }
            BookmarkDrag::Existing { .. } => {}
        }
    }

    pub(super) fn bookmark_label(&self, item: &Bookmark) -> String {
        item.label
            .clone()
            .unwrap_or_else(|| self.path_appearance(&item.path.0).0)
    }

    /// 先写入磁盘，再通知各个窗口更新同一份书签列表。
    pub(super) fn change_bookmarks(
        &mut self,
        change: impl FnOnce(&mut Bookmarks) + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.change_bookmarks_then(change, |_, _| {}, cx);
    }

    fn change_bookmarks_then(
        &mut self,
        change: impl FnOnce(&mut Bookmarks) + Send + 'static,
        complete: impl FnOnce(bool, &mut gpui_kit::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let writer = cx.global::<WorkspaceWriter>().clone();
        writer.begin_bookmark_update();
        let finish = writer.clone();
        let bookmarks = self.bookmarks.clone();
        let owner = cx.weak_entity();
        IoQueue::submit(
            cx,
            move || writer.change_bookmarks(change),
            move |result, cx| {
                finish.finish_bookmark_update();
                let error = match result {
                    Ok(saved) => {
                        bookmarks.update(cx, |current, cx| {
                            *current = saved;
                            cx.notify();
                        });
                        None
                    }
                    Err(error) => Some(format!("保存书签失败：{error}")),
                };
                let saved = error.is_none();
                let _ = owner.update(cx, |this, cx| {
                    this.workspace_error = error;
                    cx.notify();
                });
                complete(saved, cx);
            },
        );
    }

    pub(super) fn set_bookmark(
        &mut self,
        path: PathBuf,
        kind: BookmarkKind,
        present: bool,
        cx: &mut Context<Self>,
    ) {
        self.change_bookmarks(
            move |bookmarks| {
                if present {
                    bookmarks.add(path, kind);
                } else if let Some(item) = bookmarks.find(&path) {
                    bookmarks.remove(item.id);
                }
            },
            cx,
        );
    }

    pub(super) fn open_bookmark(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.bookmarks.read(cx).get(id) else {
            return;
        };
        let path = item.path.0.clone();
        match item.kind {
            BookmarkKind::Directory => self.navigate(path, window, cx),
            // 书签也可以指向链接，打开时重新判断其目标。
            BookmarkKind::File => self.open_file_target(path, EntryKind::Symlink, window, cx),
        }
    }

    pub(super) fn rename_bookmark(
        &mut self,
        id: Uuid,
        anchor_y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.bookmarks.read(cx).get(id) else {
            return;
        };
        let label = self.bookmark_label(item);
        self.close_bookmark_rename(window, cx);
        let previous_focus = window.focused(cx);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(label));
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        let submit_subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(
                event,
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false
                }
            ) {
                this.submit_bookmark_rename(window, cx);
            }
        });
        self.bookmark_rename = Some(BookmarkRename {
            id,
            input,
            position: point(SIDEBAR_WIDTH + px(8.), anchor_y),
            previous_focus,
            _submit_subscription: submit_subscription,
            saving: false,
        });
        cx.notify();
    }

    fn submit_bookmark_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self
            .bookmark_rename
            .as_mut()
            .filter(|rename| !rename.saving)
        else {
            return;
        };
        rename.saving = true;
        let id = rename.id;
        let input = rename.input.clone();
        let name = input.read(cx).value().trim().to_owned();
        let label = (!name.is_empty()).then_some(name);
        let window = window.window_handle().downcast::<Self>();
        self.change_bookmarks_then(
            move |bookmarks| bookmarks.rename(id, label),
            move |saved, cx| {
                if let Some(window) = window {
                    let _ = window.update(cx, |this, window, cx| {
                        if let Some(rename) = this
                            .bookmark_rename
                            .as_mut()
                            .filter(|rename| rename.input == input)
                        {
                            if saved {
                                this.close_bookmark_rename(window, cx);
                            } else {
                                rename.saving = false;
                                cx.notify();
                            }
                        }
                    });
                }
            },
            cx,
        );
        cx.notify();
    }

    fn close_bookmark_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(rename) = self.bookmark_rename.take() {
            if let Some(focus) = rename.previous_focus {
                window.focus(&focus, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn bookmark_rename_view(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let rename = self.bookmark_rename.as_ref()?;
        let arrow_y = rename.position.y;
        let overlay = cx.theme().popover;
        let panel = popup_surface("bookmark-rename", cx)
            .relative()
            .w(px(320.))
            .rounded(px(14.))
            .px_4()
            .py_4()
            .flex()
            .flex_col()
            .gap_2()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                this.close_bookmark_rename(window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_bookmark_rename(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("名称"))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(Input::new(&rename.input).disabled(rename.saving)),
                    )
                    .child(
                        Button::new("save-bookmark-name")
                            .primary()
                            .disabled(rename.saving)
                            .label("重命名")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit_bookmark_rename(window, cx);
                            })),
                    ),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let half = px(8.);
                        let inset = px(16.).min(bounds.size.height / 2.);
                        let center = arrow_y
                            .max(bounds.top() + inset)
                            .min(bounds.bottom() - inset);
                        let mut arrow = PathBuilder::fill();
                        arrow.move_to(point(bounds.left(), center));
                        arrow.line_to(point(bounds.right(), center - half));
                        arrow.line_to(point(bounds.right(), center + half));
                        arrow.close();
                        if let Ok(path) = arrow.build() {
                            window.paint_path(path, overlay);
                        }
                    },
                )
                .absolute()
                .left(px(-9.))
                .w(px(10.))
                .top_0()
                .bottom_0(),
            );
        Some(deferred(
            anchored()
                .position(rename.position - point(px(0.), px(44.)))
                .snap_to_window()
                .child(panel),
        ))
    }

    pub(super) fn open_bookmark_properties(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_focus = window.focused(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.bookmark_properties = Some(BookmarkProperties {
            id,
            focus,
            previous_focus,
        });
        cx.notify();
    }

    fn close_bookmark_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(properties) = self.bookmark_properties.take() {
            if let Some(focus) = properties.previous_focus {
                window.focus(&focus, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn bookmark_properties_dialog(&self, cx: &mut Context<Self>) -> Option<Dialog> {
        let properties = self.bookmark_properties.as_ref()?;
        let owner = cx.weak_entity();
        let details = self.bookmarks.read(cx).get(properties.id).map(|item| {
            let kind = if item.kind == BookmarkKind::Directory {
                "文件夹"
            } else {
                "文件"
            };
            v_flex()
                .min_w_0()
                .gap_1()
                .text_sm()
                .child(format!("名称　{}", self.bookmark_label(item)))
                .child(format!("类型　{kind}"))
                .child(format!("路径　{}", item.path.0.display()))
        });
        Some(
            Dialog::new(cx)
                .focus_handle(properties.focus.clone())
                .on_open_change(move |open, _, window, cx| {
                    if !open {
                        let _ =
                            owner.update(cx, |this, cx| this.close_bookmark_properties(window, cx));
                    }
                })
                .backdrop(div().size_full().bg(black().opacity(0.35)).occlude())
                .popup(
                    popup_surface("bookmark-properties", cx)
                        .occlude()
                        .w(px(440.))
                        .max_w(relative(0.9))
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("书签属性"))
                        .children(details)
                        .child(
                            h_flex().justify_end().child(
                                Button::new("close-bookmark-properties")
                                    .ghost()
                                    .label("关闭")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_bookmark_properties(window, cx)
                                    })),
                            ),
                        ),
                ),
        )
    }
}
