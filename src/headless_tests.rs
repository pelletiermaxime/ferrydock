//! Real GPU rendering and GPUI input dispatch with a fixture Docker boundary.
//! No compositor, Docker worker, OCR, or application state setters drive the UI.

use super::{FerryDock, Tab};
const TEXT: u32 = 0xe0ece7;
use crate::{
    compose::{self, Config as ComposeConfig, Project as ComposeProject},
    docker::{Action, Backend, Command, Connection, Event},
    model::{Container, groups},
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    AppContext, Entity, Focusable, HeadlessAppContext, InputEvent, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, SharedString, WindowHandle, px, size,
};

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use image::{
    ImageEncoder, RgbaImage,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

trait HeadlessObservation {
    fn debug_bounds(
        &mut self,
        window: gpui::AnyWindowHandle,
        selector: &str,
    ) -> anyhow::Result<Option<gpui::Bounds<gpui::Pixels>>>;
}

impl HeadlessObservation for HeadlessAppContext {
    fn debug_bounds(
        &mut self,
        window: gpui::AnyWindowHandle,
        selector: &str,
    ) -> anyhow::Result<Option<gpui::Bounds<gpui::Pixels>>> {
        self.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(gpui::SharedString::from(selector.to_owned()))
                .map(|target| target.bounds())
        })
    }
}

struct Harness {
    // Rust drops fields in declaration order. Release this external handle
    // before the context shuts down and checks for leaked entities.
    view: Entity<FerryDock>,
    cx: HeadlessAppContext,
    window: WindowHandle<gpui_kit::base::Root>,
    events: mpsc::Sender<Event>,
    commands: mpsc::UnboundedReceiver<Command>,
    evidence: PathBuf,
    actions: Vec<Value>,
    screenshots: Vec<String>,
    frame_timings: Vec<Value>,
}

impl Harness {
    fn new() -> Self {
        assert!(
            std::env::var_os("DISPLAY").is_none(),
            "Run through scripts/verify-headless.py"
        );
        assert!(
            std::env::var_os("WAYLAND_DISPLAY").is_none(),
            "Run through scripts/verify-headless.py"
        );
        let evidence = std::env::var_os("FERRYDOCK_HEADLESS_EVIDENCE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                    ".verification/headless/direct-{stamp}-{}",
                    std::process::id()
                ))
            });
        std::fs::create_dir_all(&evidence).unwrap();
        let platform = gpui_platform::current_platform(true);
        let mut cx = HeadlessAppContext::with_platform(
            platform.text_system(),
            Arc::new(gpui_kit::assets::Assets),
            || {
                // Never fall back
                // to a no-op renderer: GPU initialization failure must fail the test.
                Ok(Some(Box::new(
                    gpui_wgpu::WgpuHeadlessRenderer::new().expect("headless GPU renderer"),
                )))
            },
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            super::appearance::init_test(evidence.join("appearance.json"), cx);
        });
        let (backend, commands) = Backend::fixture();
        let (events, receiver) = mpsc::channel(256);
        let mut content = None;
        let window = cx
            .open_window(size(px(1180.0), px(820.0)), |window, cx| {
                let view = cx.new(|cx| FerryDock::with_backend(backend, receiver, window, cx));
                view.update(cx, |view, cx| view.focus(window, cx));
                content = Some(view.clone());
                cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
            })
            .unwrap();
        let view = content.unwrap();
        cx.run_until_parked();
        Self {
            cx,
            window,
            view,
            events,
            commands,
            evidence,
            actions: Vec::new(),
            screenshots: Vec::new(),
            frame_timings: Vec::new(),
        }
    }

    fn observe<R>(&self, f: impl FnOnce(&FerryDock) -> R) -> R {
        self.cx.read_entity(&self.view, |view, _| f(view))
    }

    fn notification_count(&mut self) -> usize {
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.notifications(cx).len()
            })
            .unwrap()
    }

    fn settle_ui(&mut self, milliseconds: u64) {
        for _ in 0..milliseconds.div_ceil(50) {
            // GPUI's painted animations use wall-clock frame time, while the
            // notification lifecycle uses the fixture executor's clock.
            std::thread::sleep(Duration::from_millis(50));
            self.cx.advance_clock(Duration::from_millis(50));
            self.cx.run_until_parked();
            self.cx
                .update_window(self.window.into(), |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
    }

    fn click_tab(&mut self, bar: &'static str, index: usize) {
        self.actions
            .push(json!({"input": "tab-click", "bar": bar, "index": index}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.within(bar).click(index, cx);
            })
            .unwrap();
        self.cx.run_until_parked();
        self.settle_ui(300);
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(window.within(bar).find(index).selected(), Some(true));
            })
            .unwrap();
    }

    fn inspection_value(&self) -> SharedString {
        self.cx.read_entity(&self.view, |view, cx| {
            view.containers.inspection.read(cx).value()
        })
    }

    fn dock_height(&self) -> f32 {
        self.cx.read_entity(&self.view, |view, cx| {
            f32::from(view.dock.read(cx).sizes()[1])
        })
    }

    fn drag_dock(&mut self, delta: f32) {
        self.actions
            .push(json!({"input": "dock-drag", "delta": delta}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.render_frame(cx);
                let bounds = window.find("detail-pane").bounds();
                let from = gpui::point(bounds.center().x, bounds.origin.y);
                window.drag(from, from + gpui::point(px(0.0), px(delta)), cx);
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn compose_values(&self) -> [SharedString; 4] {
        self.cx.read_entity(&self.view, |view, cx| {
            let form = view.projects.form.as_ref().expect("Open Compose form");
            [
                form.directory.read(cx).value(),
                form.lists[0].read(cx).value(),
                form.lists[1].read(cx).value(),
                form.lists[2].read(cx).value(),
            ]
        })
    }

    fn assert_compose_focus(&mut self, index: usize) {
        let view = self.view.clone();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                let form = view
                    .read(cx)
                    .projects
                    .form
                    .as_ref()
                    .expect("Open Compose form");
                let handle = if index == 0 {
                    form.directory.focus_handle(cx)
                } else {
                    form.lists[index - 1].focus_handle(cx)
                };
                assert!(
                    handle.is_focused(window),
                    "Compose field {index} must own focus"
                );
            })
            .unwrap();
    }

    fn deliver(&mut self, event: Event) {
        self.events.try_send(event).unwrap();
        self.cx.advance_clock(Duration::from_millis(40));
        self.cx.run_until_parked();
    }

    fn key(&mut self, key: &str) {
        self.actions.push(json!({"input": "key", "key": key}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.press(key, cx);
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn type_text(&mut self, text: &str) {
        self.actions.push(json!({"input": "text", "text": text}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.input(text, cx);
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn click(&mut self, selector: &str) {
        self.click_button(selector, MouseButton::Left);
    }

    fn click_button(&mut self, selector: &str, button: MouseButton) {
        // Bounds come from the painted element, never desktop coordinates.
        let bounds = self
            .cx
            .debug_bounds(self.window.into(), selector)
            .unwrap()
            .unwrap_or_else(|| panic!("No rendered control {selector}"));
        let position = bounds.center();
        self.actions.push(
            json!({"input": "click", "selector": selector, "button": format!("{button:?}"),
            "x": f32::from(position.x), "y": f32::from(position.y)}),
        );
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.dispatch_event(
                    MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Modifiers::none(),
                    }
                    .to_platform_input(),
                    cx,
                );
                // Commit hover-dependent hitboxes before pressing a control.
                window.render_frame(cx);
                window.dispatch_event(
                    MouseDownEvent {
                        position,
                        button,
                        modifiers: Modifiers::none(),
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    MouseUpEvent {
                        position,
                        button,
                        modifiers: Modifiers::none(),
                        click_count: 1,
                    }
                    .to_platform_input(),
                    cx,
                );
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn capture(&mut self, name: &str) -> RgbaImage {
        let started = Instant::now();
        let image = self.cx.capture_screenshot(self.window.into()).unwrap();
        let render_seconds = started.elapsed().as_secs_f64();
        let filename = format!("{:02}-{name}.png", self.screenshots.len() + 1);
        let encoding_started = Instant::now();
        let file =
            std::io::BufWriter::new(std::fs::File::create(self.evidence.join(&filename)).unwrap());
        // UI frames have large flat regions. Filtering plus balanced deflate
        // keeps full-resolution evidence small without a separate optimizer.
        PngEncoder::new_with_quality(file, CompressionType::Default, FilterType::Adaptive)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        self.frame_timings.push(
            json!({"screenshot": filename, "render_seconds": render_seconds,
            "encode_seconds": encoding_started.elapsed().as_secs_f64()}),
        );
        self.screenshots.push(filename);
        let scale = self
            .cx
            .update_window(self.window.into(), |_, window, _| window.scale_factor())
            .unwrap();
        assert_eq!(
            image.dimensions(),
            (
                (1180.0 * scale).round() as u32,
                (820.0 * scale).round() as u32
            )
        );
        assert!(
            image
                .pixels()
                .filter(|pixel| pixel.0[..3].iter().any(|channel| *channel > 150))
                .take(101)
                .count()
                > 100,
            "Screenshot must contain painted text/controls, not just a clear color"
        );
        std::fs::write(
            self.evidence.join("actions.json"),
            serde_json::to_vec_pretty(&self.actions).unwrap(),
        )
        .unwrap();
        image
    }

    fn drain_commands(&mut self) -> Vec<Command> {
        std::iter::from_fn(|| self.commands.try_recv().ok()).collect()
    }

    fn require_glyph_color(&mut self, image: &RgbaImage, selector: &str, color: u32) {
        let bounds = self
            .cx
            .debug_bounds(self.window.into(), selector)
            .unwrap()
            .unwrap();
        let scale = self
            .cx
            .update_window(self.window.into(), |_, window, _| window.scale_factor())
            .unwrap();
        // Styled runs blend/convert colors before entering the GPU scene. Check
        // actual pixels inside the row, excluding similarly colored controls.
        let left = (f32::from(bounds.origin.x) * scale).max(0.0) as u32;
        let top = (f32::from(bounds.origin.y) * scale).max(0.0) as u32;
        let right = (f32::from(bounds.bottom_right().x) * scale).ceil().max(0.0) as u32;
        let bottom = (f32::from(bounds.bottom_right().y) * scale).ceil().max(0.0) as u32;
        let expected = [(color >> 16) as u8, (color >> 8) as u8, color as u8];
        let mut pixels = 0;
        for y in top..bottom.min(image.height()) {
            for x in left..right.min(image.width()) {
                if image.get_pixel(x, y).0[..3] == expected {
                    pixels += 1;
                }
            }
        }
        assert!(
            pixels > 8,
            "Expected actual colored glyph pixels in {selector}: {pixels} at {bounds:?}"
        );
    }
}

fn container(id: &str, name: &str, state: &str, project: Option<&str>) -> Container {
    Container {
        id: id.into(),
        name: name.into(),
        image: "alpine:3.23".into(),
        state: state.into(),
        status: if state == "running" {
            "Up 2 minutes"
        } else {
            "Exited (0)"
        }
        .into(),
        project: project.map(str::to_owned),
        service: project.map(|_| "probe".into()),
        ports: String::new(),
        compose_directory: Default::default(),
        compose_files: vec![],
        compose_env_files: vec![],
    }
}

#[test]
fn headless_gpu_user_flow() {
    let started = Instant::now();
    let mut h = Harness::new();
    h.deliver(Event::Connected(Connection {
        name: "Headless fixture".into(),
        endpoint: "fixture://no-docker".into(),
    }));
    h.deliver(Event::Snapshot(vec![
        container(
            "fixture-probe-running",
            "headless-fixture-probe-1",
            "running",
            Some("headless-fixture"),
        ),
        container(
            "fixture-probe-stopped",
            "headless-fixture-probe-2",
            "exited",
            Some("headless-fixture"),
        ),
        container("fixture-standalone", "standalone-example", "running", None),
    ]));
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-probe-running")
    );
    let generation = h.observe(|v| v.container_logs.generation);
    h.deliver(Event::Log {
        generation,
        data: b"2026-10-12T23:30:00.000000001Z \x1b[3".to_vec(),
    });
    h.deliver(Event::Log {
        generation,
        data: b"2mHEADLESS GPU FIXTURE READY\x1b[0m\n2026-10-12T23:30:00.000000002Z A real frame, without a desktop window.\n2026-10-12T23:30:00.000000003Z \x1b[32mbuilding 10%\r\x1b[2Kdone\x1b[0m\n"
            .to_vec(),
    });
    let initial = h.capture("containers-and-logs");
    h.require_glyph_color(&initial, "container-log-line-0", crate::ansi::PALETTE[2]);
    assert_eq!(
        h.observe(|v| v.container_logs.buffer.lines[0].text.clone()),
        "HEADLESS GPU FIXTURE READY"
    );
    let probe_color = h.observe(|v| {
        v.containers
            .source_colors
            .color("headless-fixture-probe-1", 0x171d21)
    });
    h.require_glyph_color(&initial, "source-color-fixture-probe-running", probe_color);
    h.require_glyph_color(&initial, "container-log-line-1", TEXT);
    h.require_glyph_color(&initial, "container-log-line-2", crate::ansi::PALETTE[2]);
    assert!(h.observe(|v| v.container_logs.buffer.lines[2].text == "done"));
    assert!(
        !h.observe(|v| v.container_logs.buffer.text())
            .contains('\x1b')
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-stopped")
            .unwrap()
            .is_none()
    );
    assert!(
        initial
            .pixels()
            .filter(|pixel| pixel.0[..3] == [(TEXT >> 16) as u8, (TEXT >> 8) as u8, TEXT as u8])
            .count()
            > 8,
        "Real font glyphs must be painted"
    );

    // The settings UI owns theme changes; test assertions only read its state.
    let selected_before_settings = h.observe(|view| view.containers.selected.clone());
    h.click("settings");
    assert!(h.observe(|view| view.settings_open));
    h.capture("settings-ferrydock");
    h.click("theme-light");
    let light_colors =
        h.cx.update_window(h.window.into(), |_, _, cx| super::ui::palette(cx))
            .unwrap();
    assert!(
        light_colors.background > 0xd0d0d0,
        "Light theme must have a light application surface"
    );
    h.capture("settings-light");
    h.click("settings");
    let light = h.capture("containers-light");
    let light_source_color = h.observe(|v| {
        v.containers
            .source_colors
            .color("headless-fixture-probe-1", light_colors.logs)
    });
    h.require_glyph_color(
        &light,
        "source-color-fixture-probe-running",
        light_source_color,
    );
    h.require_glyph_color(&light, "container-log-line-1", light_colors.text);
    h.require_glyph_color(
        &light,
        "container-log-line-0",
        crate::ansi::display_color(crate::ansi::PALETTE[2], light_colors.logs),
    );
    assert_eq!(
        h.observe(|view| view.containers.selected.clone()),
        selected_before_settings
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(h.evidence.join("appearance.json")).unwrap())
            .unwrap();
    assert_eq!(saved["theme"], "light");
    h.click("settings");
    h.click("theme-dark");
    h.capture("settings-dark");
    h.click("settings");
    h.capture("containers-dark");
    h.key("ctrl-,");
    assert!(h.observe(|view| view.settings_open));
    h.click("theme-reset");
    h.capture("settings-default-restored");
    h.click("settings");
    let restored =
        h.cx.update_window(h.window.into(), |_, _, cx| super::ui::palette(cx))
            .unwrap();
    assert_eq!(restored.background, 0x171d21);
    assert_eq!(restored.text, TEXT);
    assert_eq!(
        h.observe(|view| view.containers.selected.clone()),
        selected_before_settings
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(h.evidence.join("appearance.json")).unwrap())
            .unwrap();
    assert_eq!(saved["theme"], "ferry-dock");
    h.click("container-more");
    h.capture("container-action-menu");
    h.key("escape");
    h.click_button("row-fixture-probe-running", MouseButton::Right);
    h.capture("container-context-menu");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.within("popup-menu").click(2, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        h.observe(|view| view.tab == Tab::Inspect),
        "Context menu Inspect must use the same navigation as the tab"
    );
    h.click_tab("container-tabs", 0);

    h.click("all");
    assert!(!h.observe(|view| view.containers.running_only));
    h.capture("containers-all");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-stopped")
            .unwrap()
            .is_some()
    );
    h.click("all");
    assert!(h.observe(|view| view.containers.running_only));

    h.click("search");
    assert!(
        h.cx.update_window(h.window.into(), |_, window, cx| {
            h.view.read(cx).containers.search_focus.is_focused(window)
        })
        .unwrap(),
        "Clicking the Input must focus its editor"
    );
    h.key("escape");
    h.key("ctrl-f");
    h.type_text("no-matching-container");
    assert_eq!(
        h.observe(|v| groups(
            &v.containers.items,
            v.containers.running_only,
            &v.containers.filter
        )
        .len()),
        0
    );
    let filtered = h.capture("empty-search");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_none()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_none()
    );
    let changed_pixels = initial
        .pixels()
        .zip(filtered.pixels())
        .filter(|(a, b)| a != b)
        .take(1001)
        .count();
    assert!(
        changed_pixels > 1000,
        "Typed search must change the GPU-rendered image"
    );

    h.key("ctrl-a");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "no-matching-container",
        "Select All must preserve the value until it is replaced"
    );
    h.type_text("dock");
    h.key("home");
    h.type_text("pre-");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "pre-dock",
        "Typing at Home must insert at the beginning"
    );
    h.key("shift-right");
    h.type_text("D");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "pre-Dock",
        "Typing must replace the selected character"
    );
    h.key("end");
    h.type_text("é");
    assert_eq!(h.observe(|v| v.containers.filter.clone()), "pre-Docké");
    h.key("backspace");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "pre-Dock",
        "Backspace must delete the complete Unicode character"
    );
    h.capture("kit-input-editing");

    h.key("ctrl-a");
    h.type_text("headless-fixture");
    h.key("escape");
    h.key("down");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "headless-fixture"
    );
    assert_eq!(
        h.observe(|v| groups(
            &v.containers.items,
            v.containers.running_only,
            &v.containers.filter
        )
        .into_values()
        .flatten()
        .count()),
        1
    );
    h.capture("filtered-fixture");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_none()
    );

    h.click("follow");
    assert!(!h.observe(|v| v.container_logs.follow));
    h.capture("logs-paused");
    h.deliver(Event::Log {
        generation,
        data: (0..200)
            .map(|index| {
                format!(
                    "SCROLLBAR ROW {index:03} {}\n",
                    "long log output ".repeat(20)
                )
            })
            .collect::<String>()
            .into_bytes(),
    });
    h.capture("logs-scrollbar-top");
    let viewport =
        h.cx.debug_bounds(h.window.into(), "container-log-viewport")
            .unwrap()
            .expect("Rendered container log viewport");
    h.actions
        .push(json!({"input": "log-scrollbar-drag", "axis": "vertical"}));
    h.cx.update_window(h.window.into(), |_, window, cx| {
        let from = gpui::point(viewport.right() - px(6.0), viewport.origin.y + px(12.0));
        let to = gpui::point(from.x, viewport.bottom() - px(20.0));
        window.drag(from, to, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.settle_ui(300);
    assert!(
        h.observe(|v| v.container_logs.scroll.0.borrow().base_handle.offset().y < px(-1000.0)),
        "Dragging the vertical scrollbar must reveal later log rows"
    );
    h.actions
        .push(json!({"input": "log-scrollbar-drag", "axis": "horizontal"}));
    h.cx.update_window(h.window.into(), |_, window, cx| {
        let from = gpui::point(viewport.origin.x + px(12.0), viewport.bottom() - px(6.0));
        let to = gpui::point(viewport.right() - px(20.0), from.y);
        window.drag(from, to, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.settle_ui(300);
    assert!(
        h.observe(|v| v.container_logs.scroll.0.borrow().base_handle.offset().x < px(-100.0)),
        "Dragging the horizontal scrollbar must reveal long log records"
    );
    h.capture("logs-scrollbar-dragged");
    h.click("follow");
    assert!(h.observe(|v| v.container_logs.follow));
    h.capture("logs-scrollbar-following");
    assert_eq!(
        h.observe(|v| v.container_logs.scroll.is_scrolled_to_end()),
        Some(true)
    );

    let initial_dock_height = h.dock_height();
    h.drag_dock(60.0);
    assert!(
        h.dock_height() < initial_dock_height - 40.0,
        "Kit divider must resize the dock through pointer input"
    );
    h.drag_dock(250.0);
    assert!(
        (h.dock_height() - 200.0).abs() < 2.0,
        "Dock minimum height must be enforced"
    );
    h.capture("kit-dock-resized");
    h.drag_dock(h.dock_height() - initial_dock_height);
    assert!((h.dock_height() - initial_dock_height).abs() < 3.0);

    h.click_tab("container-tabs", 2);
    assert!(h.observe(|view| view.tab == Tab::Inspect));
    let generation = h.observe(|view| view.container_logs.generation);
    let inspection = serde_json::to_string_pretty(&json!({
        "Id": "fixture-probe-running",
        "Config": {"Image": "alpine:3.23", "Env": ["VISIBLE=fixture-only"]},
        "HostConfig": {"NetworkMode": "bridge"}
    }))
    .unwrap();
    h.deliver(Event::Inspection {
        generation,
        result: Ok(inspection.clone()),
    });
    assert_eq!(h.inspection_value(), inspection);
    let json_image = h.capture("kit-inspect-json");
    let view = h.view.clone();
    let body =
        h.cx.update_window(h.window.into(), |_, _, cx| {
            view.read(cx)
                .containers
                .inspection
                .read(cx)
                .text_bounds()
                .expect("Painted editor text bounds")
        })
        .unwrap();
    let colored = json_image
        .enumerate_pixels()
        .filter(|(x, y, pixel)| {
            let inside = *x >= (f32::from(body.origin.x) * 2.0) as u32
                && *x < (f32::from(body.right()) * 2.0) as u32
                && *y >= (f32::from(body.origin.y) * 2.0) as u32
                && *y < (f32::from(body.bottom()) * 2.0) as u32;
            inside && pixel.0[..3].iter().max().unwrap() - pixel.0[..3].iter().min().unwrap() > 50
        })
        .count();
    assert!(
        colored > 200,
        "JSON syntax colors must be painted inside the actual editor body"
    );
    h.key("ctrl-a");
    h.type_text("must-not-edit-inspection");
    assert_eq!(
        h.inspection_value(),
        inspection,
        "Inspect must remain read-only"
    );
    h.key("ctrl-f");
    h.type_text("Image");
    h.cx.read_entity(&h.view, |view, cx| {
        let editor = view.containers.inspection.read(cx);
        let search = editor.search_session();
        assert!(search.open);
        assert!(
            !search.replace_mode,
            "Read-only search must not expose replacement"
        );
        assert_eq!(search.query, "Image");
        assert_eq!(search.matcher.len(), 1);
    });
    h.capture("kit-inspect-search");
    h.key("escape");
    h.click_tab("container-tabs", 0);
    assert!(h.observe(|view| view.tab == Tab::Logs));

    h.drain_commands();
    h.click("refresh");
    h.drain_commands();
    // Toolbar clicks preserve text focus; keyboard traversal focuses the button.
    h.key("tab");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("refresh").focused(),
            Some(true),
            "Native button must own focus for activation"
        );
    })
    .unwrap();
    for key in ["enter", "space"] {
        h.key(key);
        let commands = h.drain_commands();
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command, Command::Refresh))
                .count(),
            1,
            "Refresh activation through {key} must fire once"
        );
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::Terminal { .. })),
            "Button activation must not leak to the list shortcut"
        );
    }

    h.drain_commands();
    h.click("primary-action");
    assert!(h.drain_commands().iter().any(|command| matches!(command,
        Command::Action { id, action: Action::Stop } if id == "fixture-probe-running")));
    h.capture("stop-requested");
    h.click("container-more");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.within("popup-menu").click(7, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        h.drain_commands().is_empty(),
        "Disabled menu action must not repeat an in-flight operation"
    );
    h.key("escape");
    h.click("primary-action");
    assert!(
        h.drain_commands().is_empty(),
        "Disabled loading button must not repeat the action"
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-probe-running".into(),
        result: Ok(()),
    });
    assert_eq!(h.notification_count(), 1);
    h.settle_ui(500);
    h.capture("kit-action-notification");
    h.click("close");
    h.settle_ui(1000);
    assert_eq!(
        h.notification_count(),
        0,
        "Notification close control must dismiss feedback"
    );
    h.deliver(Event::ComposeSettingsError(
        "Fixture settings error remains visible".into(),
    ));
    h.settle_ui(500);
    h.cx.advance_clock(Duration::from_secs(6));
    h.cx.run_until_parked();
    h.settle_ui(300);
    assert_eq!(
        h.notification_count(),
        1,
        "Error feedback must remain until manually dismissed"
    );
    h.capture("kit-error-notification");
    h.click("close");
    h.settle_ui(1000);
    assert_eq!(h.notification_count(), 0);

    // Select the row before the list's Enter shortcut.
    h.click("row-fixture-probe-running");
    h.key("enter");
    assert!(h.observe(|v| v.tab == Tab::Terminal));
    let terminal_generation = h.observe(|v| v.terminal.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command,
        Command::Terminal { id, .. } if id == "fixture-probe-running")));
    h.deliver(Event::TerminalReady {
        generation: terminal_generation,
    });
    h.deliver(Event::TerminalData {
        generation: terminal_generation,
        data: b"\x1b[32mFerryDock headless GPU terminal\x1b[0m\r\n/ # ".to_vec(),
    });
    let terminal_image = h.capture("terminal-rendered");
    h.drain_commands();
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.within("shell-choice").click(1, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        h.observe(|view| matches!(view.terminal.shell, crate::docker::Shell::Auto)),
        "A connected session must disable shell selection"
    );
    h.key("ctrl-,");
    assert!(
        h.observe(|view| view.settings_open),
        "Settings shortcut must work from a focused terminal"
    );
    h.click("theme-light");
    h.click("settings");
    h.capture("terminal-light");
    assert_eq!(
        h.observe(|view| view.terminal.generation),
        terminal_generation
    );
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::Terminal { .. } | Command::CloseTerminal)),
        "Appearance changes must not recreate or close a live shell"
    );
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    h.type_text("echo verified");
    h.key("enter");
    let input: Vec<u8> = h
        .drain_commands()
        .into_iter()
        .filter_map(|command| match command {
            Command::Input { data, .. } => Some(data),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(input, b"echo verified\r");

    let log_generation = h.observe(|v| v.container_logs.generation);
    let terminal_text = h.cx.read_entity(&h.view, |v, cx| {
        v.terminal
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .core
            .visible_text()
    });
    let inspection = h.inspection_value();
    let logs = h.observe(|v| v.container_logs.buffer.text());
    h.deliver(Event::Disconnected("Fixture Docker disconnected".into()));
    let log_status = h.observe(|v| v.container_logs.status.clone());
    let terminal_status = h.cx.read_entity(&h.view, |v, cx| {
        v.terminal.view.as_ref().unwrap().read(cx).status.clone()
    });
    for event in [
        Event::TerminalReady {
            generation: terminal_generation,
        },
        Event::TerminalData {
            generation: terminal_generation,
            data: b"STALE TERMINAL OUTPUT".to_vec(),
        },
        Event::TerminalEnded {
            generation: terminal_generation,
            error: Some("Stale shell error".into()),
        },
        Event::Log {
            generation: log_generation,
            data: b"STALE LOG OUTPUT\n".to_vec(),
        },
        Event::LogsEnded {
            generation: log_generation,
            error: Some("Stale log error".into()),
        },
        Event::Inspection {
            generation: log_generation,
            result: Ok("Stale inspection".into()),
        },
    ] {
        h.deliver(event);
        assert!(
            h.cx.read_entity(&h.view, |v, cx| !v
                .terminal
                .view
                .as_ref()
                .unwrap()
                .read(cx)
                .ready),
            "Disconnected shells must ignore stale ready events"
        );
    }
    assert_eq!(
        h.cx.read_entity(&h.view, |v, cx| v
            .terminal
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .core
            .visible_text()),
        terminal_text
    );
    assert_eq!(
        h.cx.read_entity(&h.view, |v, cx| v
            .terminal
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .status
            .clone()),
        terminal_status
    );
    assert_eq!(h.observe(|v| v.container_logs.buffer.text()), logs);
    assert_eq!(h.observe(|v| v.container_logs.status.clone()), log_status);
    assert_eq!(h.inspection_value(), inspection);
    h.capture("disconnected-stale-events-ignored");
    h.deliver(Event::Connected(Connection {
        name: "Fixture Docker".into(),
        endpoint: "fixture://docker".into(),
    }));
    h.deliver(Event::Snapshot(h.observe(|v| v.containers.items.clone())));
    h.drain_commands();

    h.deliver(Event::ComposeProjects(vec![ComposeProject {
        name: "headless-fixture".into(),
        config: None,
        hint: ComposeConfig {
            project: "headless-fixture".into(),
            directory: "/fixture/project".into(),
            files: vec!["compose.yaml".into()],
            ..Default::default()
        },
    }]));
    h.click("project-headless-fixture");
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::ProjectLogs { project, .. } if project == "headless-fixture")));
    let generation = h.observe(|v| v.project_logs.generation);
    let mut members = h.observe(|v| v.containers.items.clone());
    let mut worker = container(
        "fixture-worker",
        "headless-fixture-worker-1",
        "running",
        Some("headless-fixture"),
    );
    worker.service = Some("worker".into());
    members.push(worker.clone());
    h.deliver(Event::Snapshot(members));
    for (id, service, name, text) in [
        (
            "fixture-probe-running",
            "probe",
            "headless-fixture-probe-1",
            "2026-10-12T23:30:00.000000001Z \x1b[32mHTTP ready\x1b[0m",
        ),
        (
            "fixture-worker",
            "worker",
            "headless-fixture-worker-1",
            "2026-10-12T23:30:00.000000002Z Queue ready",
        ),
    ] {
        h.deliver(Event::ProjectLog {
            generation,
            line: crate::project_logs::Line {
                container: id.into(),
                service: service.into(),
                name: name.into(),
                text: text.into(),
                error: false,
            },
        });
    }
    h.deliver(Event::ProjectLogsStatus {
        generation,
        status: "3 project containers · 2 live streams · merged as received".into(),
    });
    h.click("project-more");
    h.capture("project-action-menu");
    h.key("escape");
    h.click("project-log-follow");
    assert!(!h.observe(|v| v.project_logs.follow));
    let project_image = h.capture("project-logs-all");
    h.require_glyph_color(
        &project_image,
        "project-log-line-0",
        crate::ansi::PALETTE[2],
    );
    let worker_color = h.observe(|v| {
        v.containers
            .source_colors
            .color("headless-fixture-worker-1", 0x101819)
    });
    assert_ne!(probe_color, worker_color);
    h.require_glyph_color(&project_image, "project-log-line-0", probe_color);
    h.require_glyph_color(&project_image, "project-log-line-1", worker_color);
    h.require_glyph_color(
        &project_image,
        "source-color-fixture-probe-running",
        probe_color,
    );
    h.require_glyph_color(&project_image, "source-color-fixture-worker", worker_color);
    h.require_glyph_color(&project_image, "project-log-line-1", TEXT);
    assert!(
        !h.observe(|v| v.project_logs.history.text(None))
            .contains('\x1b')
    );
    assert!(
        !h.observe(|v| v.project_logs.history.text(None))
            .contains(".00000000")
    );
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    let project_light = h.capture("project-logs-light");
    let light_background =
        h.cx.update_window(h.window.into(), |_, _, cx| super::ui::palette(cx).logs)
            .unwrap();
    let light_probe = h.observe(|v| {
        v.containers
            .source_colors
            .color("headless-fixture-probe-1", light_background)
    });
    let light_worker = h.observe(|v| {
        v.containers
            .source_colors
            .color("headless-fixture-worker-1", light_background)
    });
    h.require_glyph_color(&project_light, "project-log-line-0", light_probe);
    h.require_glyph_color(&project_light, "project-log-line-1", light_worker);
    h.require_glyph_color(
        &project_light,
        "source-color-fixture-probe-running",
        light_probe,
    );
    h.require_glyph_color(&project_light, "source-color-fixture-worker", light_worker);
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    h.click("project-service-filter");
    h.type_text("probe");
    h.key("enter");
    assert_eq!(
        h.observe(|v| v.project_logs.service.clone()).as_deref(),
        Some("probe")
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-log-line-0")
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-log-line-1")
            .unwrap()
            .is_none()
    );
    h.click("project-log-copy");
    assert_eq!(
        h.notification_count(),
        1,
        "Copy feedback must use a Kit notification"
    );
    let filtered = h.capture("project-logs-filtered");
    h.require_glyph_color(&filtered, "project-log-line-0", probe_color);
    h.click_tab("project-tabs", 1);
    assert!(h.observe(|view| view.projects.tab == super::projects::ProjectTab::Operations));
    h.click_tab("project-tabs", 0);
    assert!(h.observe(|view| view.projects.tab == super::projects::ProjectTab::Logs));
    h.click("project-service-filter");
    h.type_text("All services");
    h.key("enter");
    h.click("project-log-follow");
    assert!(h.observe(|v| v.project_logs.follow));
    let mut members = h.observe(|v| v.containers.items.clone());
    members.retain(|container| container.id != "fixture-worker");
    worker.id = "fixture-worker-recreated".into();
    members.push(worker);
    h.deliver(Event::Snapshot(members));
    h.deliver(Event::ProjectLog {
        generation,
        line: crate::project_logs::Line {
            container: "fixture-worker-recreated".into(),
            service: "worker".into(),
            name: "headless-fixture-worker-1".into(),
            text: "2026-10-12T23:30:01.000000001Z Worker recreated and live".into(),
            error: false,
        },
    });
    assert!(h.observe(|v| {
        v.project_logs
            .history
            .text(None)
            .contains("Worker recreated and live")
    }));
    let recreated = h.capture("project-logs-recreated");
    h.require_glyph_color(&recreated, "project-log-line-2", worker_color);
    h.require_glyph_color(
        &recreated,
        "source-color-fixture-worker-recreated",
        worker_color,
    );
    h.click("row-fixture-probe-running");
    let commands = h.drain_commands();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::CloseProjectLogs))
    );
    assert!(commands.iter().any(
        |command| matches!(command, Command::Logs { id, .. } if id == "fixture-probe-running")
    ));
    h.key("ctrl-4");
    h.key("ctrl-1");
    assert!(h.observe(|v| v.projects.selected.is_none() && v.tab == Tab::Logs));
    let timestamp = "2026-10-13T05:00:00.000000001Z";
    h.deliver(Event::Log {
        generation: h.observe(|v| v.container_logs.generation),
        data: format!("{timestamp} \x1b[32mbuilding 10%\r\x1b[2Kdone\x1b[0m\n").into_bytes(),
    });
    assert_eq!(
        h.observe(|v| v.container_logs.buffer.text()),
        "done",
        "Returning from project tabs must keep Docker timestamps hidden across ANSI progress updates"
    );
    h.capture("container-logs-after-project-tab");
    h.key("ctrl-4");
    h.key("ctrl-1");
    h.drain_commands();
    // The worker publishes the complete initial history in one event. Its first
    // painted frame must already contain the tail, without an earlier top frame.
    h.deliver(Event::Log {
        generation: h.observe(|v| v.container_logs.generation),
        data: (0..500)
            .map(|index| format!("2026-10-13T05:00:00.{index:09}Z INITIAL HISTORY {index:03}\n"))
            .collect::<String>()
            .into_bytes(),
    });
    h.capture("logs-open-at-bottom");
    assert_eq!(
        h.observe(|v| v.container_logs.scroll.is_scrolled_to_end()),
        Some(true)
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "container-log-line-499")
            .unwrap()
            .is_some(),
        "The newest initial log record must appear in the first frame"
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "container-log-line-0")
            .unwrap()
            .is_none(),
        "Initial history must not paint from the top before reaching the tail"
    );
    h.key("ctrl-4");
    assert_eq!(
        h.observe(|v| v.projects.selected.clone()).as_deref(),
        Some("headless-fixture")
    );
    h.drain_commands();
    h.click("compose-start");
    h.click("compose-rebuild");
    assert!(
        h.drain_commands().is_empty(),
        "Unassociated project must not run"
    );
    h.click("compose-configure");
    h.assert_compose_focus(0);
    let initial_directory = h.compose_values()[0].clone();
    h.key("ctrl-a");
    assert_eq!(
        h.compose_values()[0],
        initial_directory,
        "Select All must not clear the directory"
    );
    h.type_text("/fixture/associated");
    h.key("home");
    h.type_text("/temporary");
    assert_eq!(h.compose_values()[0], "/temporary/fixture/associated");
    h.key("ctrl-z");
    assert_eq!(
        h.compose_values()[0],
        "/fixture/associated",
        "Undo must restore the directory"
    );
    h.key("end");
    h.type_text("é");
    h.key("backspace");
    assert_eq!(h.compose_values()[0], "/fixture/associated");
    h.key("tab");
    h.assert_compose_focus(1);
    h.key("ctrl-a");
    h.type_text("base.yaml");
    h.key("enter");
    h.type_text("override.yaml");
    h.key("home");
    h.key("shift-right");
    h.type_text("O");
    assert_eq!(
        h.compose_values()[1],
        "base.yaml\nOverride.yaml",
        "Textarea must replace selected text at the caret"
    );
    h.key("ctrl-z");
    assert_eq!(h.compose_values()[1], "base.yaml\noverride.yaml");
    h.key("shift-tab");
    h.assert_compose_focus(0);
    h.key("tab");
    h.assert_compose_focus(1);
    h.key("tab");
    h.assert_compose_focus(2);
    h.type_text(".env.dev");
    h.key("tab");
    h.assert_compose_focus(3);
    h.type_text("dev");
    h.key("tab");
    h.assert_compose_focus(0);
    h.key("shift-tab");
    h.assert_compose_focus(3);
    h.click("compose-field-1");
    h.assert_compose_focus(1);
    h.key("ctrl-a");
    assert_eq!(h.compose_values()[1], "base.yaml\noverride.yaml");
    h.capture("compose-association");
    h.click("compose-save");
    let saving_values = h.compose_values();
    h.type_text("must-not-change-while-saving");
    h.key("escape");
    assert_eq!(
        h.compose_values(),
        saving_values,
        "Saving must disable input and cancellation"
    );
    let config = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::ConfigureCompose { config } => Some(config),
            _ => None,
        })
        .expect("Association command");
    assert_eq!(config.directory, PathBuf::from("/fixture/associated"));
    assert_eq!(
        config.files,
        vec![PathBuf::from("base.yaml"), PathBuf::from("override.yaml")]
    );
    assert_eq!(config.env_files, vec![PathBuf::from(".env.dev")]);
    assert_eq!(config.profiles, vec!["dev"]);
    h.deliver(Event::ComposeConfigured {
        project: config.project.clone(),
        result: Ok(config),
    });
    h.settle_ui(500);
    h.drain_commands();
    h.click("compose-configure");
    h.key("ctrl-a");
    h.type_text("/discarded-edit");
    h.key("escape");
    assert!(
        h.observe(|view| view.projects.form.is_none()),
        "Escape must cancel an editable form"
    );
    assert!(
        h.drain_commands().is_empty(),
        "Cancellation must not save changes"
    );
    h.click("compose-start");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action: compose::Action::Start,
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Compose Start request");
    h.click("compose-stop");
    h.click("compose-rebuild");
    assert!(
        h.drain_commands().is_empty(),
        "Conflicting controls must be disabled"
    );
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"\x1b[33mContainer headless-fixture-probe-1 Starting".to_vec(),
    });
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"\r\x1b[2K\x1b[32mContainer headless-fixture-probe-1 Started\x1b[0m\n".to_vec(),
    });
    let operation_image = h.capture("compose-operation-live");
    h.require_glyph_color(
        &operation_image,
        "compose-log-line-0",
        crate::ansi::PALETTE[2],
    );
    assert!(h.observe(|v| {
        let output = v.projects.operations["headless-fixture"].output.text();
        !output.contains('\x1b') && !output.contains("Starting") && output.contains("Started")
    }));
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Ok(()),
    });
    assert!(h.observe(|v| {
        v.projects.operations["headless-fixture"]
            .result
            .as_ref()
            .unwrap()
            .is_ok()
    }));
    h.click("compose-stop");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                operation,
                action: compose::Action::Stop,
                ..
            } => Some(operation),
            _ => None,
        })
        .expect("Compose Stop request");
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"Fixture Compose failure output retained\n".to_vec(),
    });
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Err("Fixture Compose operation failed".into()),
    });
    h.capture("compose-operation-failed");
    assert!(h.observe(|v| {
        !v.project_busy("headless-fixture")
            && v.projects.operations["headless-fixture"]
                .output
                .text()
                .contains("failure output retained")
    }));
    h.click("compose-restart");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                operation,
                action: compose::Action::Restart,
                ..
            } => Some(operation),
            _ => None,
        })
        .expect("Compose Restart request");
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Ok(()),
    });
    h.click("compose-rebuild");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action: compose::Action::Rebuild,
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Compose Rebuild & restart request");
    assert!(h.observe(|v| v.projects.tab == super::projects::ProjectTab::Operations));
    for control in [
        "compose-start",
        "compose-stop",
        "compose-restart",
        "compose-rebuild",
        "compose-configure",
    ] {
        h.click(control);
    }
    assert!(
        h.drain_commands().is_empty(),
        "Rebuild must block duplicate and conflicting actions"
    );
    h.key("ctrl-1");
    h.drain_commands();
    h.click("primary-action");
    assert!(
        h.drain_commands().is_empty(),
        "Container controls must be blocked during a project rebuild"
    );
    h.key("ctrl-4");
    h.click_tab("project-tabs", 1);
    h.drain_commands();
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"Building probe image\nContainer headless-fixture-probe-1 Recreated\n".to_vec(),
    });
    h.capture("compose-rebuild-live");
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Err("Fixture build failed".into()),
    });
    h.key("ctrl-1");
    h.key("ctrl-4");
    h.click_tab("project-tabs", 1);
    assert!(
        h.observe(|v| {
            let operation = &v.projects.operations["headless-fixture"];
            !operation.running
                && operation.result.as_ref().unwrap().is_err()
                && operation.output.text().contains("Building probe image")
        }),
        "Build failure and output must survive leaving the project and returning"
    );
    h.capture("compose-rebuild-failed");
    h.drain_commands();
    h.click("project-more");
    h.capture("compose-rebuild-menu");
    h.actions.push(json!({"input":"menu-click", "menu":"project-more", "index":6, "action":"Rebuild & restart"}));
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.within("popup-menu").click(6, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action: compose::Action::Rebuild,
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Project menu Rebuild & restart request");
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"Probe image built\nConfigured services recreated and started\n".to_vec(),
    });
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Ok(()),
    });
    assert!(h.observe(|v| {
        let operation = &v.projects.operations["headless-fixture"];
        !v.project_busy("headless-fixture") && operation.result.as_ref().unwrap().is_ok()
    }));
    h.capture("compose-rebuild-completed");

    h.click_tab("project-tabs", 0);
    let generation = h.observe(|v| v.project_logs.generation);
    let ingestion_started = Instant::now();
    for chunk in (0..1500).collect::<Vec<_>>().chunks(64) {
        for index in chunk {
            h.events
                .try_send(Event::ProjectLog {
                    generation,
                    line: crate::project_logs::Line {
                        container: format!("stress-{}", index % 10),
                        service: format!("service-{}", index % 10),
                        name: format!("stress-service-{}-1", index % 10),
                        text: format!(
                            "2026-10-12T23:30:02.000000001Z busy-service log line {index:04} {}",
                            "application output ".repeat(8)
                        ),
                        error: false,
                    },
                })
                .unwrap();
        }
        h.cx.advance_clock(Duration::from_millis(40));
        h.cx.run_until_parked();
    }
    let ingestion_seconds = ingestion_started.elapsed().as_secs_f64();
    h.capture("project-logs-busy");
    let rows_rendered = h.observe(|v| v.project_logs.rows_rendered.get());
    assert!(
        rows_rendered < 80,
        "Log layout must be limited to visible rows, not all 1,500 retained lines"
    );
    assert!(
        h.observe(|v| v.project_logs.scroll.is_scrolled_to_end()) == Some(true),
        "Follow must jump directly to the current tail"
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-log-line-1499")
            .unwrap()
            .is_some()
    );
    let batched_started = Instant::now();
    for chunk in (0..10_000)
        .collect::<Vec<_>>()
        .chunks(crate::project_logs::MAX_BATCH_LINES)
    {
        h.events
            .try_send(Event::ProjectLogBatch {
                generation,
                lines: chunk
                    .iter()
                    .map(|index| crate::project_logs::Line {
                        container: format!("stress-{}", index % 10),
                        service: format!("service-{}", index % 10),
                        name: format!("stress-service-{}-1", index % 10),
                        text: format!(
                            "2026-10-12T23:30:03.000000001Z batched busy log line {index:04}"
                        ),
                        error: false,
                    })
                    .collect(),
            })
            .unwrap();
    }
    for _ in 0..20 {
        h.cx.advance_clock(Duration::from_millis(40));
        h.cx.run_until_parked();
        if h.observe(|v| {
            v.project_logs
                .history
                .lines
                .back()
                .is_some_and(|line| line.text.contains("line 9999"))
        }) {
            break;
        }
    }
    let batched_ingestion_seconds = batched_started.elapsed().as_secs_f64();
    assert!(h.observe(|v| {
        v.project_logs.history.lines.len() == 1500
            && v.project_logs
                .history
                .lines
                .back()
                .unwrap()
                .text
                .contains("line 9999")
    }));
    h.capture("project-logs-batched-busy");
    assert!(h.observe(|v| v.project_logs.rows_rendered.get()) < 80);
    assert_eq!(
        h.observe(|v| v.project_logs.scroll.is_scrolled_to_end()),
        Some(true)
    );
    h.click("project-log-follow");
    assert!(
        !h.observe(|v| v.project_logs.follow),
        "Controls must remain responsive after a log flood"
    );
    let performance = json!({"input_lines":1500,"retained_lines":h.observe(|v| v.project_logs.history.lines.len()),"rows_rendered_in_frame":rows_rendered,"ingestion_seconds":ingestion_seconds,
        "batched_input_lines":10000,"batched_ingestion_seconds":batched_ingestion_seconds,"batched_rows_rendered":h.observe(|v| v.project_logs.rows_rendered.get()),"follow_reached_tail":true,"controls_respond_after_flood":true});
    std::fs::write(
        h.evidence.join("log-performance.json"),
        serde_json::to_vec_pretty(&performance).unwrap(),
    )
    .unwrap();

    let report = json!({
        "status": "passed", "renderer": "GPUI Kit gpui-pre WgpuHeadlessRenderer",
        "display": null, "wayland_display": null, "docker_worker_started": false,
        "logical_width": 1180, "logical_height": 820,
        "pixel_width": terminal_image.width(), "pixel_height": terminal_image.height(),
        "render_and_flow_seconds": started.elapsed().as_secs_f64(),
        "changed_pixels_after_search_lower_bound": changed_pixels, "screenshots": h.screenshots,
        "frame_timings": h.frame_timings,
        "log_performance": performance,
        "checks": ["fixture discovery", "Kit appearance Settings pointer theme switching and default restoration", "theme preference persistence in isolated file", "light theme log glyph contrast", "settings preserve selected resource", "Kit container action menu opens and dismisses", "Kit right-click Inspect menu navigation", "Kit project action menu and disabled unassociated controls", "Kit Checkbox running filter and stopped rows", "Kit menu action blocks duplicate operation", "Kit RadioGroup shell choice disabled during live session", "theme switching preserves terminal generation and input focus", "Kit service Select search and keyboard confirmation", "search pointer focus", "keyboard search", "Kit Input select all and replacement", "Kit Input caret navigation and selection", "Kit Input Unicode backspace", "empty matches", "restored selection",
            "Kit Button Tab focus and Enter Space activation without shortcut leakage", "Kit Button disabled loading action guard",
            "Kit container and project TabBar pointer selection", "Kit Resizable pointer drag and minimum height",
            "Kit Inspect read-only typing guard", "Kit Inspect JSON syntax glyph colors", "Kit Inspect search without replacement",
            "Kit success notification and visible close control", "Kit error notification persists beyond success timeout",
            "container ANSI glyph colors and split sequences", "matching container markers and distinct project source colors", "source markers readable in Light theme", "project ANSI payload styles and plain copy without Docker timestamps", "Compose ANSI colors and carriage-return progress replacement",
            "log follow pointer toggle", "stop command at Docker boundary", "terminal ANSI rendering", "terminal input routing",
            "disconnection ignores stale terminal, log and inspection events", "project tab return keeps Docker timestamps hidden",
            "Compose association keyboard form", "Kit Compose caret selection and Unicode editing", "Kit Compose undo",
            "Kit Compose pointer focus and Tab Shift-Tab wraparound", "Kit Compose disabled editing while saving", "Kit Compose Escape cancellation",
            "ordered files, environment files and profiles", "unassociated and conflicting actions blocked",
            "Compose Start Stop Restart Rebuild requests", "Rebuild toolbar and project menu requests", "Rebuild duplicate and conflicting project/container actions blocked", "Rebuild live output and success", "Rebuild failure output retained after leaving and returning", "streamed operation output and retained failure",
            "project log request without association", "combined service prefixes", "project service filter rendered rows",
            "project log follow pause and copy feedback", "recreated member log rendering", "return to container log routing"]
    });
    std::fs::write(
        h.evidence.join("headless.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    std::fs::write(
        h.evidence.join("actions.json"),
        serde_json::to_vec_pretty(&h.actions).unwrap(),
    )
    .unwrap();
    println!("Headless GPU evidence: {}", h.evidence.display());
}
