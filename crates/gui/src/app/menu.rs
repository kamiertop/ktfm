use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, menu::PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Pixels, SharedString, WeakEntity, Window, px};

use super::App;

// 在组件默认间距上微调图标与文字的距离。
pub(super) const MENU_ICON_EXTRA_GAP: Pixels = px(3.);

pub(super) fn app_menu_item(
    owner: &WeakEntity<App>,
    label: impl Into<SharedString>,
    icon: impl Into<Option<IconName>>,
    action: impl Fn(&mut App, &mut Window, &mut Context<App>) + 'static,
) -> PopupMenuItem {
    let mut item = PopupMenuItem::new(label);
    if let Some(icon) = icon.into() {
        item = item.icon(Icon::new(icon).mr(MENU_ICON_EXTRA_GAP));
    }
    let owner = owner.clone();
    let action = std::rc::Rc::new(action);
    item.on_click(move |_, window, cx| {
        let owner = owner.clone();
        let action = action.clone();
        // 菜单关闭并归还焦点后，再执行可能打开窗口或编辑框的动作。
        window.defer(cx, move |window, cx| {
            let _ = owner.update(cx, |app, cx| action(app, window, cx));
        });
    })
}
