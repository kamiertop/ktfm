mod app;
mod assets;
mod components;
mod model;
mod style;

use gpui_kit::assets::AllAssets;
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::WARN.into())
                .from_env_lossy(),
        )
        .with_writer(std::io::stderr)
        .init();
    tracing::debug!("启动 ktfm");

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
