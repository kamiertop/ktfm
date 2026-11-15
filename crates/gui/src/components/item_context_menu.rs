use ely_core::menus::{ContextMenu, Menu};
use gpui_kit::{IntoElement, Pixels, Point, Styled, px};

#[derive(Default)]
pub struct ItemContextMenu {
    sequence: u64,
    request: Option<(Point<Pixels>, Menu)>,
}

impl ItemContextMenu {
    /// 在右键落下时创建菜单，避免为每个可见卡片提前构造操作列表。
    pub fn open(&mut self, position: Point<Pixels>, menu: Menu) {
        self.sequence = self.sequence.wrapping_add(1);
        self.request = Some((position, menu));
    }

    pub fn clear(&mut self) {
        self.request = None;
    }

    pub fn element(&self) -> Option<impl IntoElement + use<>> {
        let (position, menu) = self.request.as_ref()?;
        // 同一个窗口共用一个 Ely 宿主。请求编号变化时，菜单才会重新打开。
        Some(
            ContextMenu::new("file-item-menu", menu.clone())
                .manual(Some((self.sequence, *position)))
                .absolute()
                .top(px(0.))
                .left(px(0.))
                .size(px(0.)),
        )
    }
}
