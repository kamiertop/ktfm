use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants};

pub fn icon_button(id: &'static str, icon: IconName, label: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .icon(Icon::new(icon))
        .accessibility_label(label)
}

pub fn with_tooltip(button: Button, label: &'static str) -> impl gpui_kit::IntoElement {
    use gpui_kit::component::tooltip::Tooltip;
    use gpui_kit::div;
    use gpui_kit::prelude::*;

    div()
        .id(label)
        .flex_shrink_0()
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .child(button)
}
