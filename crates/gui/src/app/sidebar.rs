use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::tab::Location;

use super::App;
use super::tags::tag_color;
use crate::components::icon_button;
use crate::style::{SIDEBAR_ICON_SIZE, SIDEBAR_WIDTH, icon_color, sidebar_divider_color};

impl App {
    pub(super) fn is_default_favorite(&self, path: &Path) -> bool {
        self.default_favorites
            .iter()
            .any(|item| item.path.as_deref() == Some(path))
    }

    pub(super) fn toggle_star(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.is_default_favorite(&path) {
            return;
        }
        if let Some(item) = self.workspace.stars.find(&path) {
            self.workspace.stars.remove(item.id);
        } else {
            self.workspace.stars.add(path, None);
        }
        self.save_workspace(cx);
    }

    pub(super) fn sidebar(&self, radius: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, border, foreground, danger) = (
            theme.sidebar,
            theme.sidebar_border,
            theme.sidebar_foreground,
            theme.danger,
        );
        let entity = cx.entity().downgrade();
        let location = self.location();
        let favorites = self
            .workspace
            .stars
            .items()
            .iter()
            .filter(|item| !self.is_default_favorite(&item.path.0))
            .map(|item| {
                let id = item.id;
                let path = item.path.0.clone();
                let (name, _) = self.path_appearance(&path);
                let entity = entity.clone();
                sidebar_row(
                    SharedString::from(format!("favorite-{id}")),
                    name,
                    IconName::Star,
                    item.color.map(tag_color),
                    Some(path.as_path()) == location,
                    cx,
                )
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this, _, window, cx| this.navigate(path.clone(), window, cx)),
                )
                .context_menu(move |menu, _, _| {
                    let entity = entity.clone();
                    menu.item(PopupMenuItem::new("取消收藏").on_click(move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.workspace.stars.remove(id);
                            this.save_workspace(cx);
                        });
                    }))
                })
            });

        let tags = self.tags.items().iter().map(|tag| {
            let id = tag.id;
            let location = Location::Tag { id };
            let selected = self.current_location() == Some(&location);
            let entity = entity.clone();
            sidebar_row(
                SharedString::from(format!("tag-{id}")),
                tag.name.clone(),
                IconName::Tag,
                tag.color.map(tag_color),
                selected,
                cx,
            )
            .cursor_pointer()
            .on_click(
                cx.listener(move |this, _, window, cx| this.visit(location.clone(), window, cx)),
            )
            .context_menu(move |menu, _, _| {
                let open = entity.clone();
                let edit = entity.clone();
                let delete = entity.clone();
                menu.item(
                    PopupMenuItem::new("在新标签页打开").on_click(move |_, window, cx| {
                        let _ = open.update(cx, |this, cx| {
                            this.new_tab(Some(Location::Tag { id }), window, cx)
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("编辑标签").on_click(move |_, window, cx| {
                        let _ =
                            edit.update(cx, |this, cx| this.edit_tag(Some(id), None, window, cx));
                    }),
                )
                .item(
                    PopupMenuItem::new("删除标签").on_click(move |_, window, cx| {
                        let _ = delete.update(cx, |this, cx| this.delete_tag(id, window, cx));
                    }),
                )
            })
        });

        let volumes = self.storage.volumes.iter().map(|volume| {
            let is_mounting = self.storage.mounting.as_ref() == Some(&volume.device);
            let label = if is_mounting {
                format!("{}（挂载中）", volume.label)
            } else {
                volume.label.clone()
            };
            let active = volume.mountpoint.is_some() && volume.mountpoint.as_deref() == location;
            let row = sidebar_row(
                SharedString::from(format!("volume-{}", volume.device.display())),
                label,
                if volume.removable {
                    IconName::Usb
                } else {
                    IconName::HardDrive
                },
                None,
                active,
                cx,
            );
            let volume = volume.clone();
            row.cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_volume(&volume, window, cx);
                }))
        });

        let content = v_flex()
            .size_full()
            .px_2()
            .py_3()
            .gap_2()
            .child(
                v_flex()
                    .gap_0p5()
                    .children(self.default_favorites.iter().map(|item| {
                        sidebar_row(
                            item.name,
                            item.name.to_owned(),
                            item.icon,
                            None,
                            item.path.is_some() && item.path.as_deref() == location,
                            cx,
                        )
                        .when(item.path.is_none(), |row| row.opacity(0.5))
                        .when_some(item.path.clone(), |row, path| {
                            row.cursor_pointer().on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.navigate(path.clone(), window, cx);
                                },
                            ))
                        })
                    }))
                    .children(favorites),
            )
            .child(
                section(cx)
                    .child(
                        h_flex()
                            .justify_between()
                            .px_1()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("标签"),
                            )
                            .child(icon_button("new-tag", IconName::Plus, "新建标签").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.edit_tag(None, None, window, cx)
                                }),
                            )),
                    )
                    .children(tags),
            )
            .when(
                !self.storage.volumes.is_empty()
                    || self.storage.error.is_some()
                    || self.storage.discovery_error.is_some(),
                |content| {
                    content.child(
                        section(cx).children(volumes).when_some(
                            self.storage
                                .error
                                .clone()
                                .or_else(|| self.storage.discovery_error.clone()),
                            |group, message| {
                                group
                                    .child(div().px_1().text_sm().text_color(danger).child(message))
                            },
                        ),
                    )
                },
            )
            .child(
                section(cx).children(
                    [("网络", IconName::Network), ("回收站", IconName::Trash)]
                        .into_iter()
                        .map(|(name, icon)| {
                            sidebar_row(name, name.to_owned(), icon, None, false, cx).opacity(0.5)
                        }),
                ),
            )
            .overflow_y_scrollbar();

        div()
            .w(SIDEBAR_WIDTH)
            .min_h_0()
            .flex_shrink_0()
            .rounded_bl(radius)
            .bg(background)
            .text_color(foreground)
            .child(
                div()
                    .size_full()
                    .border_r_1()
                    .border_color(border)
                    .child(content),
            )
    }
}

fn sidebar_row(
    id: impl Into<ElementId>,
    name: String,
    icon: IconName,
    color: Option<Hsla>,
    active: bool,
    cx: &gpui_kit::App,
) -> Stateful<Div> {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .h_8()
        .px_1p5()
        .gap_2()
        .rounded_md()
        .text_base()
        .when(active, |row| row.bg(theme.sidebar_accent).font_semibold())
        .when(!active, |row| row.hover(|style| style.bg(theme.list_hover)))
        .child(
            Icon::new(icon)
                .size(SIDEBAR_ICON_SIZE)
                .flex_shrink_0()
                .text_color(color.unwrap_or_else(|| icon_color(icon, theme.mode))),
        )
        .child(div().min_w_0().truncate().child(name))
}

fn section(cx: &gpui_kit::App) -> Div {
    let theme = cx.theme();
    v_flex().gap_0p5().child(
        div()
            .mx_1()
            .mb_1()
            .h(px(1.))
            .bg(sidebar_divider_color(theme.mode)),
    )
}
