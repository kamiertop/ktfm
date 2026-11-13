use gpui_kit::component::Icon;

pub fn window_restore() -> Icon {
    Icon::default().data(include_bytes!("../../../assets/icons/window_restore.svg"))
}
