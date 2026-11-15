use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{ActiveTheme, Icon, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::command::AppCommand;
use settings::config::tab::{Location, Session, TabItem};
use settings::config::workspace::Workspace;
use settings::tag::Tag;
use uuid::Uuid;

use super::App;
use super::preferences::ActivePreferences;
use super::workspace::WorkspaceWriter;
use crate::components::{icon_button, truncated_text, with_tooltip};
use crate::style::SIDEBAR_WIDTH;

const TAB_BAR_PADDING: Pixels = px(8.);
const TAB_OUTLINE_WIDTH: Pixels = px(1.5);

actions!(browser, [NewTab, CloseTab]);

pub(super) struct BrowserTab {
    id: Uuid,
    history: Vec<Location>,
    index: usize,
}

impl BrowserTab {
    pub(super) fn new(id: Uuid, location: Option<Location>) -> Self {
        Self {
            id,
            history: location.into_iter().collect(),
            index: 0,
        }
    }

    fn location(&self) -> Option<&Location> {
        self.history.get(self.index)
    }

    fn visit(&mut self, location: Location) {
        self.history.truncate(self.index + 1);
        self.history.push(location);
        self.index = self.history.len() - 1;
    }
}

pub(super) fn restore_tabs(
    workspace: &Workspace,
    tags: Option<&Tag>,
    home: Option<PathBuf>,
) -> (Vec<BrowserTab>, usize) {
    let mut tabs = Vec::new();
    for item in &workspace.session.tabs {
        let valid = match &item.location {
            Location::Directory { path } => path.0.is_absolute(),
            Location::Tag { id } => tags.is_none_or(|tags| tags.get(*id).is_some()),
        };
        if valid && !tabs.iter().any(|tab: &BrowserTab| tab.id == item.id) {
            tabs.push(BrowserTab::new(item.id, Some(item.location.clone())));
        }
    }
    if tabs.is_empty() {
        tabs.push(BrowserTab::new(
            Uuid::new_v4(),
            home.map(Location::directory),
        ));
    }
    let active = tabs
        .iter()
        .position(|tab| Some(tab.id) == workspace.session.active)
        .unwrap_or(0);
    (tabs, active)
}

impl App {
    pub(super) fn current_location(&self) -> Option<&Location> {
        self.tabs[self.active_tab].location()
    }

    pub(super) fn location(&self) -> Option<&Path> {
        match self.current_location()? {
            Location::Directory { path } => Some(&path.0),
            Location::Tag { .. } => None,
        }
    }

    pub(super) fn location_appearance(
        &self,
        location: Option<&Location>,
        cx: &gpui_kit::App,
    ) -> (String, IconName) {
        match location {
            Some(Location::Directory { path }) => self.path_appearance(&path.0),
            Some(Location::Tag { id }) => (
                self.tags(cx)
                    .get(*id)
                    .map_or_else(|| "已删除的标签".into(), |tag| tag.name.clone()),
                IconName::Tag,
            ),
            None => ("主文件夹".into(), IconName::House),
        }
    }

    pub(super) fn navigate(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.visit(Location::directory(path), window, cx);
    }

    pub(super) fn visit(
        &mut self,
        location: Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.current_location() == Some(&location) {
            return;
        }
        self.tabs[self.active_tab].visit(location);
        self.location_changed(window, cx);
    }

    pub(super) fn can_go_back(&self) -> bool {
        self.tabs[self.active_tab].index > 0
    }

    pub(super) fn can_go_forward(&self) -> bool {
        let tab = &self.tabs[self.active_tab];
        tab.index + 1 < tab.history.len()
    }

    pub(super) fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.can_go_back() {
            self.tabs[self.active_tab].index -= 1;
            self.location_changed(window, cx);
        }
    }

    pub(super) fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.can_go_forward() {
            self.tabs[self.active_tab].index += 1;
            self.location_changed(window, cx);
        }
    }

    pub(super) fn new_tab(
        &mut self,
        location: Option<Location>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.push(BrowserTab::new(Uuid::new_v4(), location));
        self.active_tab = self.tabs.len() - 1;
        self.location_changed(window, cx);
    }

    fn switch_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() && index != self.active_tab {
            self.active_tab = index;
            self.location_changed(window, cx);
        }
    }

    pub(super) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        match index.cmp(&self.active_tab) {
            std::cmp::Ordering::Equal => {
                self.active_tab = index.min(self.tabs.len() - 1);
                self.location_changed(window, cx);
                return;
            }
            std::cmp::Ordering::Less => self.active_tab -= 1,
            std::cmp::Ordering::Greater => {}
        }
        self.save_workspace(cx);
    }

    pub(super) fn forget_tag(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.forget_tab_locations(|location| *location == Location::Tag { id }, window, cx);
    }

    pub(super) fn forget_missing_tags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.tag_state.clone();
        let Some(tags) = state.read(cx).loaded_tags() else {
            return;
        };
        let missing = self
            .tabs
            .iter()
            .flat_map(|tab| &tab.history)
            .filter_map(|location| match location {
                Location::Tag { id } if tags.get(*id).is_none() => Some(*id),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        if !missing.is_empty() {
            self.forget_tab_locations(
                |location| matches!(location, Location::Tag { id } if missing.contains(id)),
                window,
                cx,
            );
        }
    }

    fn forget_tab_locations(
        &mut self,
        removed: impl Fn(&Location) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_active = self.current_location().is_some_and(&removed);
        let home = self.home().map(|path| Location::directory(path.to_owned()));
        for tab in &mut self.tabs {
            let before = tab.history[..tab.index]
                .iter()
                .filter(|item| !removed(item))
                .count();
            tab.history.retain(|item| !removed(item));
            if tab.history.is_empty() {
                tab.history.extend(home.clone());
            }
            tab.index = before.min(tab.history.len().saturating_sub(1));
        }
        if was_active {
            self.location_changed(window, cx);
        } else {
            self.save_workspace(cx);
        }
    }

    fn location_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigation_revision += 1;
        self.browser.query.clear();
        self.search.update(cx, |search, cx| {
            search.set_value("", window, cx);
        });
        self.load_directory(cx);
        self.save_workspace(cx);
    }

    pub(super) fn save_workspace(&mut self, cx: &mut Context<Self>) {
        // 只有主窗口的标签页写入会话。
        if self.persist_session && self.workspace_writable {
            let session = Session {
                tabs: self
                    .tabs
                    .iter()
                    .filter_map(|tab| {
                        Some(TabItem {
                            id: tab.id,
                            location: tab.location()?.clone(),
                        })
                    })
                    .collect(),
                active: Some(self.tabs[self.active_tab].id),
            };
            cx.global::<WorkspaceWriter>()
                .clone()
                .save_session(session, cx);
        }
        cx.notify();
    }

    pub(super) fn open_new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_tab(self.current_location().cloned(), window, cx);
    }

    pub(super) fn show_tab_bar(&self) -> bool {
        self.tabs.len() > 1
    }

    pub(super) fn new_tab_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let label: SharedString = format!(
            "新建标签页 ({})",
            cx.global::<ActivePreferences>().0.shortcuts.new_tab
        )
        .into();
        with_tooltip(
            icon_button("new-tab", IconName::Plus, "新建标签页")
                .on_click(cx.listener(|this, _, window, cx| this.open_new_tab(window, cx))),
            label,
        )
    }

    pub(super) fn tab_bar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (foreground, muted) = (theme.foreground, theme.muted_foreground);
        let outline = rgb(if theme.mode.is_dark() {
            0x94839e
        } else {
            0xa391af
        });
        let available =
            window.viewport_size().width - SIDEBAR_WIDTH - TAB_BAR_PADDING * 2. - px(2.);
        let share = available.max(px(0.)) / self.tabs.len() as f32;
        let show_icon = share >= px(68.);
        let show_close = share >= px(116.);
        let divider = rgb(if theme.mode.is_dark() {
            0x74697e
        } else {
            0xb2a6ba
        })
        .into();
        let active_text = rgb(if theme.mode.is_dark() {
            0xe0c5f5
        } else {
            0x65417c
        });
        h_flex()
            .id("browser-tabs")
            .relative()
            .flex_shrink_0()
            .min_w_0()
            .px(TAB_BAR_PADDING)
            .overflow_hidden()
            .child(tab_outline(
                self.active_tab,
                self.tabs.len(),
                outline,
                divider,
            ))
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                let (name, icon) = self.location_appearance(tab.location(), cx);
                let tooltip: SharedString = match tab.location() {
                    Some(Location::Directory { path }) => {
                        path.0.to_string_lossy().into_owned().into()
                    }
                    _ => name.clone().into(),
                };
                let selected = index == self.active_tab;
                let id = tab.id;
                h_flex()
                    .id(SharedString::from(id.to_string()))
                    .relative()
                    .h_9()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(if selected { foreground } else { muted })
                    .when(!selected, |tab| {
                        tab.hover(|style| style.text_color(foreground).font_semibold())
                    })
                    .when(selected, |tab| tab.text_color(active_text).font_semibold())
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| {
                            match cx
                                .global::<ActivePreferences>()
                                .0
                                .actions
                                .middle_click_tab
                                .to_command()
                            {
                                Some(AppCommand::CloseCurrentTab) => {
                                    this.close_tab(index, window, cx)
                                }
                                Some(AppCommand::DuplicateCurrentTab) => {
                                    if let Some(tab) = this.tabs.get(index) {
                                        this.new_tab(tab.location().cloned(), window, cx);
                                    }
                                }
                                _ => {}
                            }
                        }),
                    )
                    .child(
                        h_flex()
                            .id("select-tab")
                            .size_full()
                            .min_w_0()
                            .justify_center()
                            .px(if show_close { px(38.) } else { px(7.) })
                            .gap_1p5()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.switch_tab(index, window, cx);
                            }))
                            .when(show_icon, |label| {
                                label.child(Icon::new(icon).size(px(15.)).flex_shrink_0())
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .child(truncated_text(name.into(), tooltip)),
                            ),
                    )
                    .when(show_close, |tab| {
                        tab.child(
                            h_flex()
                                .id("close-tab")
                                .absolute()
                                .right(px(15.))
                                .top(px(7.))
                                .size(px(22.))
                                .justify_center()
                                .rounded_full()
                                .cursor_pointer()
                                .hover(move |style| {
                                    style.bg(active_text.opacity(0.13)).text_color(active_text)
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.close_tab(index, window, cx);
                                }))
                                .child(Icon::new(IconName::X).size(px(14.))),
                        )
                    })
            }))
    }
}

fn tab_outline(active: usize, count: usize, outline: Rgba, divider: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let width = bounds.size.width;
            let share = (width - TAB_BAR_PADDING * 2.) / count as f32;
            if share <= px(0.) {
                return;
            }
            let left = TAB_BAR_PADDING + share * active as f32;
            let right = left + share;
            let top = px(1.); // 距离顶部 1px，避免与标题栏边框重叠
            let bottom = bounds.size.height - TAB_OUTLINE_WIDTH / 2.;
            let radius = px(9.).min(share / 4.).min((bottom - top) / 2.);
            let radii = point(radius, radius);
            let at = |x, y| bounds.origin + point(x, y);

            let mut path = PathBuilder::stroke(TAB_OUTLINE_WIDTH);
            if active == 0 {
                path.move_to(at(px(0.), top));
            } else {
                path.move_to(at(px(0.), bottom));
                path.line_to(at(left, bottom));
                path.arc_to(
                    radii,
                    px(0.),
                    false,
                    false,
                    at(left + radius, bottom - radius),
                );
                path.line_to(at(left + radius, top + radius));
                path.arc_to(radii, px(0.), false, true, at(left + radius * 2., top));
            }

            if active + 1 == count {
                path.line_to(at(width, top));
            } else {
                path.line_to(at(right - radius * 2., top));
                path.arc_to(radii, px(0.), false, true, at(right - radius, top + radius));
                path.line_to(at(right - radius, bottom - radius));
                path.arc_to(radii, px(0.), false, false, at(right, bottom));
                path.line_to(at(width, bottom));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, outline);
            }

            let middle = bounds.size.height / 2.;
            let mut separators = PathBuilder::stroke(px(1.));
            for boundary in 1..count {
                if boundary != active && boundary != active + 1 {
                    let x = TAB_BAR_PADDING + share * boundary as f32;
                    separators.move_to(at(x, middle - px(9.)));
                    separators.line_to(at(x, middle + px(9.)));
                }
            }
            if let Ok(path) = separators.build() {
                window.paint_path(path, divider);
            }
        },
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::{BrowserTab, Location, TabItem, Tag, Uuid, Workspace, restore_tabs};

    #[test]
    fn restores_valid_tabs_and_falls_back_when_the_active_tag_is_missing() {
        let valid = TabItem {
            id: Uuid::new_v4(),
            location: Location::directory("/home".into()),
        };
        let missing = TabItem {
            id: Uuid::new_v4(),
            location: Location::Tag { id: Uuid::new_v4() },
        };
        let mut workspace = Workspace::default();
        workspace.session.active = Some(missing.id);
        workspace.session.tabs = vec![missing, valid.clone(), valid.clone()];
        let (tabs, active) = restore_tabs(&workspace, Some(&Tag::default()), Some("/".into()));
        assert_eq!(active, 0);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[active].location(), Some(&valid.location));
    }

    #[test]
    fn visiting_after_back_replaces_only_that_tabs_forward_history() {
        let mut tab = BrowserTab::new(Uuid::new_v4(), Some(Location::directory("/home".into())));
        let other = BrowserTab::new(Uuid::new_v4(), Some(Location::directory("/tmp".into())));
        tab.visit(Location::directory("/home/old".into()));
        tab.index = 0;
        let target = Location::Tag { id: Uuid::new_v4() };
        tab.visit(target.clone());
        assert_eq!(tab.history.len(), 2);
        assert_eq!(tab.location(), Some(&target));
        assert_eq!(other.location(), Some(&Location::directory("/tmp".into())));
    }
}
