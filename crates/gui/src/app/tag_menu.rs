use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable as _, actions::Cancel};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use uuid::Uuid;

use super::App as FileManager;
use super::file_item_actions::MENU_ICON_EXTRA_GAP;

#[derive(Clone, Copy)]
enum TagAction {
    New,
    Set { id: Uuid, present: bool },
}

struct TagMenuRow {
    label: SharedString,
    icon: Option<IconName>,
    action: TagAction,
    focus: FocusHandle,
}

struct TagMenu {
    owner: WeakEntity<FileManager>,
    path: PathBuf,
    menu_focus: FocusHandle,
    rows: Vec<TagMenuRow>,
    fixed_rows: usize,
    scroll: ScrollHandle,
    _keys: Subscription,
}

pub(super) fn build_tag_menu(
    menu: PopupMenu,
    owner: WeakEntity<FileManager>,
    path: PathBuf,
    current_tag: Option<Uuid>,
    tags: &[(Uuid, String, bool)],
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let menu_focus = menu.focus_handle(cx);
    let content = cx.new(|cx| {
        let mut rows = Vec::with_capacity(tags.len() + 2);
        let row = |label, icon, action| TagMenuRow {
            label,
            icon,
            action,
            focus: cx.focus_handle().tab_stop(true),
        };
        rows.push(row(
            "新建并添加".into(),
            Some(IconName::Plus),
            TagAction::New,
        ));
        if let Some(id) = current_tag {
            rows.push(row(
                "移除".into(),
                Some(IconName::Minus),
                TagAction::Set { id, present: false },
            ));
        }
        let fixed_rows = rows.len();
        rows.extend(tags.iter().map(|(id, name, checked)| {
            row(
                name.clone().into(),
                checked.then_some(IconName::Check),
                TagAction::Set {
                    id: *id,
                    present: !checked,
                },
            )
        }));

        // 子菜单接管方向键，焦点离开后让其他菜单继续使用组件的默认键位。
        let weak = cx.weak_entity();
        let focus = menu_focus.clone();
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            if !focus.contains_focused(window, cx)
                || event.keystroke.modifiers.modified()
                || !matches!(event.keystroke.key.as_str(), "up" | "down" | "enter")
            {
                return;
            }
            let _ = weak.update(cx, |menu: &mut TagMenu, cx| {
                menu.handle_key(event.keystroke.key.as_str(), window, cx);
                cx.stop_propagation();
            });
        });
        TagMenu {
            owner,
            path,
            menu_focus,
            rows,
            fixed_rows,
            scroll: ScrollHandle::default(),
            _keys: keys,
        }
    });

    menu.min_w(px(124.))
        .max_w(px(156.))
        .item(PopupMenuItem::element(move |_, _| content.clone()).disabled(true))
}

impl TagMenu {
    fn handle_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let index = self
            .rows
            .iter()
            .position(|row| row.focus.is_focused(window))
            .unwrap_or(0);
        if key == "enter" {
            self.activate(self.rows[index].action, window, cx);
            return;
        }
        let next = if key == "up" {
            index.checked_sub(1).unwrap_or(self.rows.len() - 1)
        } else {
            (index + 1) % self.rows.len()
        };
        if let Some(index) = next.checked_sub(self.fixed_rows) {
            self.scroll.scroll_to_item(index);
        }
        window.focus(&self.rows[next].focus, cx);
        cx.notify();
    }

    fn activate(&mut self, action: TagAction, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.menu_focus, cx);
        window.dispatch_action(Box::new(Cancel), cx);
        let _ = self.owner.update(cx, |app, cx| match action {
            TagAction::New => app.edit_tag(None, Some(self.path.clone()), window, cx),
            TagAction::Set { id, present } => app.set_tag_path(id, self.path.clone(), present, cx),
        });
    }
}

impl Render for TagMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = (window.viewport_size().height * 0.45).min(px(196.));
        let icon_gap = rems(0.25).to_pixels(window.rem_size()) + MENU_ICON_EXTRA_GAP;
        let row = |(index, row): (usize, &TagMenuRow), cx: &mut Context<Self>| {
            let action = row.action;
            Button::new(("tag-menu-row", index))
                .ghost()
                .track_focus(&row.focus)
                .w_full()
                .h(rems(1.625))
                .flex_shrink_0()
                .p_0()
                .accessibility_label(row.label.clone())
                .tooltip(row.label.clone())
                .toggled(
                    matches!(action, TagAction::Set { present: false, .. })
                        && index >= self.fixed_rows,
                )
                .selected(
                    row.focus.is_focused(window)
                        || (index == 0 && self.menu_focus.is_focused(window)),
                )
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap(icon_gap)
                        .child(
                            div()
                                .w_3()
                                .flex_shrink_0()
                                .children(row.icon.map(|icon| Icon::new(icon).xsmall())),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(row.label.clone()),
                        ),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.activate(action, window, cx);
                    cx.stop_propagation();
                }))
        };
        let mut content = v_flex().flex_1().min_w_0().children(
            self.rows
                .iter()
                .enumerate()
                .take(self.fixed_rows)
                .map(|entry| row(entry, cx)),
        );
        if self.rows.len() > self.fixed_rows {
            let list = v_flex()
                .id("tag-menu-list")
                .w_full()
                .min_w_0()
                .max_h(height)
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .children(
                    self.rows
                        .iter()
                        .enumerate()
                        .skip(self.fixed_rows)
                        .map(|entry| row(entry, cx)),
                );
            content = content
                .child(div().my_0p5().border_t_1().border_color(cx.theme().border))
                .child(list.vertical_scrollbar(&self.scroll));
        }
        content
    }
}
