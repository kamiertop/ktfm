use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::div;
use gpui_kit::prelude::*;

pub fn icon_button(id: &'static str, icon: IconName, label: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .icon(Icon::new(icon))
        .accessibility_label(label)
}

pub fn with_tooltip(
    button: Button,
    label: impl Into<gpui_kit::SharedString>,
) -> impl gpui_kit::IntoElement {
    let label = label.into();
    div()
        .id(label.clone())
        .flex_shrink_0()
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .child(button)
}

/// 将提示锚定到按钮下方，鼠标进入按钮的位置不会影响提示高度。
pub fn with_anchored_tooltip(
    id: &'static str,
    button: Button,
    label: &'static str,
) -> impl gpui_kit::IntoElement {
    AnchoredTooltipButton { id, button, label }
}

#[derive(gpui_kit::IntoElement)]
struct AnchoredTooltipButton {
    id: &'static str,
    button: Button,
    label: &'static str,
}

impl gpui_kit::RenderOnce for AnchoredTooltipButton {
    fn render(
        self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::App,
    ) -> impl gpui_kit::IntoElement {
        use std::{cell::Cell, rc::Rc};

        use gpui_kit::base::{ElementExt as _, Placement};
        use gpui_kit::base::{TooltipOverlay, TooltipRequest};
        use gpui_kit::{Bounds, ElementId, MouseButton, Pixels};

        let overlay = window.use_keyed_state((ElementId::from(self.id), "tooltip"), cx, |_, _| {
            TooltipOverlay::new()
        });
        let hover_overlay = overlay.clone();
        let press_overlay = overlay.clone();
        let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
        let anchor = bounds.clone();
        let label = self.label;

        div()
            .id((ElementId::from(self.id), "trigger"))
            .flex_shrink_0()
            .on_prepaint(move |bounds, _, _| anchor.set(bounds))
            .on_hover(move |hovered, window, cx| {
                hover_overlay.update(cx, |overlay, cx| {
                    if *hovered {
                        let request = TooltipRequest::new(bounds.get(), move |window, cx| {
                            Tooltip::new(label).build(window, cx)
                        })
                        .with_placement(Placement::Bottom);
                        overlay.request_show(request, window, cx);
                    } else {
                        overlay.request_hide(window, cx);
                    }
                });
            })
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                press_overlay.update(cx, |overlay, cx| overlay.hide(cx));
            })
            .child(self.button)
            .child(div().absolute().child(overlay))
    }
}
