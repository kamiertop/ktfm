mod app;
mod assets;
mod components;
mod model;
mod style;

use gpui_kit::assets::AllAssets;
use gpui_kit::component::TitleBar;
use gpui_kit::{
    AppContext, KeyBinding, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowOptions, size,
};

use style::{MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, WINDOW_HEIGHT, WINDOW_WIDTH};

fn main() {
    gpui_kit::application().with_assets(AllAssets).run(|cx| {
        gpui_kit::init(cx);
        style::install_themes(cx);
        cx.bind_keys([KeyBinding::new("ctrl-t", app::NewTab, Some("FileManager"))]);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(
                size(WINDOW_WIDTH, WINDOW_HEIGHT),
                cx,
            )),
            window_min_size: Some(size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)),
            window_background: WindowBackgroundAppearance::Transparent,
            window_decorations: Some(WindowDecorations::Client),
            ..TitleBar::window_options()
        };

        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        // 直接挂载应用视图，由应用统一绘制窗口背景、圆角和外框。
        if let Err(error) =
            cx.open_window(options, |window, cx| cx.new(|cx| app::App::new(window, cx)))
        {
            eprintln!("无法打开窗口：{error:#}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
