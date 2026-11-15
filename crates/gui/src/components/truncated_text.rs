use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{InteractiveText, SharedString, StyledText};

pub fn truncated_text(label: SharedString, tooltip: SharedString) -> InteractiveText {
    let text = StyledText::new(label.clone());
    let layout = text.layout().clone();

    // 比较实际排版结果，避免按字符数估算文字宽度。
    InteractiveText::new("truncated-text", text).tooltip(move |_, window, cx| {
        (layout.text().as_str() != label.as_ref())
            .then(|| Tooltip::new(tooltip.clone()).build(window, cx))
    })
}
