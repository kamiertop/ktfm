use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Div, InteractiveElement, Stateful, Styled, div};

pub fn popup_surface(id: &'static str, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id(id)
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .shadow_md()
}
