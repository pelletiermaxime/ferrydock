use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

fn main() {
    #[cfg(target_os = "linux")]
    {
        if std::env::var("DISPLAY").map_or(true, |display| display.is_empty()) {
            eprintln!(
                "FerryDock needs an X11 or XWayland desktop connection (DISPLAY). Launch it from your desktop terminal."
            );
            std::process::exit(1);
        }
        // The published CE platform enables Wayland transitively and prefers it
        // when both display variables are present. Keep this build on X11/XWayland.
        // SAFETY: this runs before GPUI or the Docker worker starts any threads.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
    gpui_platform::application().run(|cx: &mut App| {
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1180.0), px(820.0)), cx);
        cx.open_window(
            WindowOptions::new()
                .window_bounds(Some(WindowBounds::Windowed(bounds)))
                .titlebar(Some(
                    TitlebarOptions::default().title(Some("FerryDock".into())),
                ))
                .app_id(Some("dev.ferrydock.FerryDock".into()))
                .window_min_size(Some(size(px(820.0), px(600.0)))),
            |window, cx| {
                let view = cx.new(ferrydock::app::FerryDock::new);
                view.update(cx, |view, cx| view.focus(window, cx));
                view
            },
        )
        .expect("could not open FerryDock window");
        cx.activate(true);
    });
}
