mod app;
mod assets;
mod components;
mod model;
mod style;

use gpui_kit::KeyBinding;
use gpui_kit::assets::AllAssets;

fn main() {
    gpui_kit::application().with_assets(AllAssets).run(|cx| {
        gpui_kit::init(cx);
        style::install_themes(cx);

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
