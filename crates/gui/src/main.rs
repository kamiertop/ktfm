mod app;
mod assets;
mod components;
mod model;
mod style;

use gpui_kit::assets::AllAssets;

fn main() {
    gpui_kit::application().with_assets(AllAssets).run(|cx| {
        gpui_kit::init(cx);
        style::install_themes(cx);

        cx.on_window_closed(|cx, _| {
            app::FileJobs::quit_if_idle(cx);
        })
        .detach();

        app::App::open_main_window(cx);
    });
}
