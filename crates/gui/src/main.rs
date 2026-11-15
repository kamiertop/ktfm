mod app;
mod assets;
mod components;
mod model;
mod style;

use gpui_kit::KeyBinding;
use gpui_kit::assets::AllAssets;

fn main() {
    gpui_kit::application()
        .with_assets(ely_core::Assets::before(AllAssets))
        .run(|cx| {
            gpui_kit::init(cx);
            if let Err(error) = ely_core::init(cx) {
                eprintln!("无法初始化 Ely 菜单：{error:#}");
                cx.quit();
                return;
            }
            style::install_themes(cx);
            style::sync_menu_theme(cx);
            cx.observe_global::<gpui_kit::component::Theme>(style::sync_menu_theme)
                .detach();

            cx.bind_keys([KeyBinding::new(
                "ctrl-p",
                app::OpenPreferences,
                Some("FileManager"),
            )]);

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            app::App::open_main_window(cx);
        });
}
