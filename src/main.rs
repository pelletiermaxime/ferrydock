#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

fn main() {
    #[cfg(all(feature = "hot-reload", debug_assertions))]
    env_logger::init();

    #[cfg(target_os = "linux")]
    {
        let has_display = ["WAYLAND_DISPLAY", "DISPLAY"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
        if !has_display {
            eprintln!(
                "FerryDock needs a Wayland (WAYLAND_DISPLAY) or X11 (DISPLAY) desktop connection. Launch it from your desktop terminal."
            );
            std::process::exit(1);
        }
    }
    gpui_kit::application()
        .with_assets(ferrydock::assets::Assets)
        .run(|cx: &mut App| {
            ferrydock::app::init_ui(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1180.0), px(820.0)), cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("FerryDock".into()),
                        ..Default::default()
                    }),
                    app_id: Some("dev.ferrydock.FerryDock".into()),
                    window_min_size: Some(size(px(820.0), px(640.0))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let view = cx.new(|cx| ferrydock::app::FerryDock::new(window, cx));
                    view.update(cx, |view, cx| view.focus(window, cx));
                    view
                },
            )
            .expect("could not open FerryDock window");
            cx.activate(true);
        });
}
