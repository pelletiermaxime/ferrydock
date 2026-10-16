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
        Self::with_size(1180.0, 820.0)
    }

    fn with_size(width: f32, height: f32) -> Self {
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
            Arc::new(crate::assets::Assets),
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
            .open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| {
                    FerryDock::with_backend(
                        backend,
                        receiver,
                        super::favorites::Favorites::with_path(Ok(evidence.join("favorites.json"))),
                        window,
                        cx,
                    )
                });
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

    fn drag_list_scrollbar(&mut self, viewport: &'static str, to_end: bool) {
        self.actions
            .push(json!({"input": "list-scrollbar-drag", "viewport": viewport, "to_end": to_end}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.render_frame(cx);
                let bounds = window.find(viewport).bounds();
                let top = gpui::point(bounds.right() - px(6.0), bounds.top() + px(12.0));
                let bottom = gpui::point(top.x, bounds.bottom() - px(12.0));
                let (from, to) = if to_end { (top, bottom) } else { (bottom, top) };
                window.drag(from, to, cx);
            })
            .unwrap();
        self.cx.run_until_parked();
        self.settle_ui(300);
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
        if (selector == "image-remove" && self.observe(|v| v.images.confirmation.is_some()))
            || (selector == "image-prune" && self.observe(|v| v.images.prune_review))
            || (selector == "volume-remove" && self.observe(|v| v.volumes.confirmation.is_some()))
            || (selector == "volume-prune" && self.observe(|v| v.volumes.prune_review))
        {
            self.settle_ui(300);
        }
    }

    fn dismiss_image_feedback(&mut self) {
        self.settle_ui(300);
        if self.notification_count() > 0 {
            self.click("close");
            self.settle_ui(300);
        }
    }

    fn hover(&mut self, selector: &str, horizontal: f32) {
        let bounds = self
            .cx
            .debug_bounds(self.window.into(), selector)
            .unwrap()
            .unwrap();
        let position = gpui::point(
            bounds.left() + bounds.size.width * horizontal,
            bounds.top() + bounds.size.height * 0.4,
        );
        self.actions
            .push(json!({"input": "hover", "selector": selector, "fraction": horizontal}));
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
                window.render_frame(cx);
            })
            .unwrap();
        self.cx.run_until_parked();
        self.settle_ui(300);
    }

    fn scroll_containers(&mut self, delta: f32) {
        let bounds = self
            .cx
            .debug_bounds(self.window.into(), "containers-pane")
            .unwrap()
            .unwrap();
        self.actions
            .push(json!({"input": "wheel", "selector": "containers-pane", "delta": delta}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.dispatch_event(
                    gpui::ScrollWheelEvent {
                        position: bounds.center(),
                        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(delta))),
                        modifiers: Modifiers::none(),
                        touch_phase: gpui::TouchPhase::Moved,
                    }
                    .to_platform_input(),
                    cx,
                );
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn history_click_line(&mut self, line: usize, button: MouseButton) {
        self.click_button(&format!("history-row-{line}"), button);
    }

    fn history_scroll(&mut self, x: f32, y: f32) {
        let bounds = self
            .cx
            .debug_bounds(self.window.into(), "image-history-table")
            .unwrap()
            .unwrap();
        self.actions
            .push(json!({"input":"history wheel", "x":x,"y":y}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.dispatch_event(
                    gpui::ScrollWheelEvent {
                        position: bounds.center(),
                        delta: gpui::ScrollDelta::Pixels(gpui::point(px(x), px(y))),
                        modifiers: Modifiers::none(),
                        touch_phase: gpui::TouchPhase::Moved,
                    }
                    .to_platform_input(),
                    cx,
                );
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn tooltip_label(&mut self, selector: &'static str) -> String {
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.find(selector).label().unwrap().to_owned()
            })
            .unwrap()
    }

    fn click_button(&mut self, selector: &str, button: MouseButton) {
        self.click_with_modifiers(selector, button, Modifiers::none());
    }

    fn ctrl_click(&mut self, selector: &str) {
        self.click_with_modifiers(
            selector,
            MouseButton::Left,
            Modifiers {
                control: true,
                ..Modifiers::none()
            },
        );
    }

    fn click_with_modifiers(&mut self, selector: &str, button: MouseButton, modifiers: Modifiers) {
        // Bounds come from the painted element, never desktop coordinates.
        let bounds = self
            .cx
            .debug_bounds(self.window.into(), selector)
            .unwrap()
            .unwrap_or_else(|| panic!("No rendered control {selector}"));
        let position = bounds.center();
        self.actions.push(
            json!({"input": "click", "selector": selector, "button": format!("{button:?}"), "control":modifiers.control,
            "x": f32::from(position.x), "y": f32::from(position.y)}),
        );
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.dispatch_event(
                    MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers,
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
                        modifiers,
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
                        modifiers,
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
        let (scale, viewport) = self
            .cx
            .update_window(self.window.into(), |_, window, _| {
                (window.scale_factor(), window.viewport_size())
            })
            .unwrap();
        assert_eq!(
            image.dimensions(),
            (
                (f32::from(viewport.width) * scale).round() as u32,
                (f32::from(viewport.height) * scale).round() as u32
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
    let original_containers = h.observe(|v| v.containers.items.clone());
    h.click("all");
    h.capture("compose-groups-expanded");
    h.click("all");
    h.drain_commands();
    h.click("toggle-compose-headless-fixture");
    h.capture("compose-group-collapsed");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_none()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-probe-running")
    );
    assert!(h.observe(|v| v.projects.selected.is_none()));
    assert!(
        h.drain_commands().is_empty(),
        "Folding must preserve the detail session"
    );
    h.deliver(Event::Snapshot(original_containers.clone()));
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_none()
    );

    // Search temporarily reveals folded matches without discarding the fold.
    h.key("ctrl-f");
    h.type_text("headless-fixture");
    h.capture("compose-group-search-revealed");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_some()
    );
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_none()
    );
    h.key("down");
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-standalone")
    );
    h.click("toggle-standalone");
    h.capture("all-container-groups-collapsed");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_none()
    );
    h.key("up");
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-standalone")
    );
    h.click("toggle-standalone");
    h.click("toggle-compose-headless-fixture");
    h.click("row-fixture-probe-running");
    let mut many_containers = original_containers.clone();
    many_containers.extend((0..100).map(|index| {
        container(
            &format!("scroll-fixture-{index:03}"),
            &format!("scroll-fixture-{index:03}"),
            "running",
            Some("scroll-fixture"),
        )
    }));
    h.deliver(Event::Snapshot(many_containers));
    h.capture("containers-scrollbar-top");
    h.drag_list_scrollbar("containers-viewport", true);
    assert!(h.observe(|v| v.containers.scroll.0.borrow().base_handle.offset().y < px(-1000.0)));
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-probe-running")
    );
    h.capture("containers-scrollbar-dragged");
    h.drag_list_scrollbar("containers-viewport", false);
    assert!(h.observe(|v| v.containers.scroll.0.borrow().base_handle.offset().y > px(-100.0)));
    h.deliver(Event::Snapshot(original_containers));
    h.deliver(Event::MetricsSession(1));
    h.capture("metrics-collecting");
    for index in 0..60 {
        h.deliver(Event::Metrics {
            session: 1,
            id: "fixture-probe-running".into(),
            result: Ok(crate::metrics::Sample {
                collected: Instant::now() - Duration::from_secs((59 - index) * 2),
                cpu_percent: Some(12.0 + ((index * 7) % 23) as f64 * 8.0),
                memory_used: Some((144 + (index % 12) * 4) * 1024 * 1024),
                memory_capacity: Some(512 * 1024 * 1024),
                limits: Some(crate::metrics::Limits {
                    memory: Some(512 * 1024 * 1024),
                    cpu_cores: Some(2.0),
                    cpuset: None,
                }),
                reset: false,
            }),
        });
    }
    let last_sample = h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .samples
            .back()
            .unwrap()
            .clone()
    });
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-standalone".into(),
        result: Ok(crate::metrics::Sample {
            limits: Some(crate::metrics::Limits::default()),
            memory_capacity: Some(8 * 1024 * 1024 * 1024),
            ..last_sample.clone()
        }),
    });
    h.deliver(Event::Metrics {
        session: 0,
        id: "fixture-probe-running".into(),
        result: Err("Old session".into()),
    });
    assert!(h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .live()
            .is_some()
    }));
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Err("Stats unavailable".into()),
    });
    assert!(h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .live()
            .is_none()
    }));
    h.capture("metrics-unavailable");
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Ok(crate::metrics::Sample {
            collected: Instant::now() - crate::metrics::STALE_AFTER - Duration::from_secs(1),
            ..last_sample.clone()
        }),
    });
    assert!(h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .live()
            .is_none()
    }));
    h.capture("metrics-stale");
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Ok(crate::metrics::Sample {
            collected: Instant::now(),
            ..last_sample
        }),
    });
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
    h.require_glyph_color(&initial, "cpu-history", 0x74d7b1);
    h.require_glyph_color(&initial, "memory-history", 0x74d7b1);
    h.require_glyph_color(&initial, "metrics-row-fixture-probe-running", 0x9eb2a8);
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
    h.hover("cpu-history", 0.35);
    let cpu_hover = h.capture("cpu-chart-hover");
    let first_cpu_label = h.tooltip_label("cpu-hover-value");
    assert!(first_cpu_label.starts_with("CPU: ") && first_cpu_label.ends_with('%'));
    h.require_glyph_color(&cpu_hover, "cpu-hover-value", TEXT);
    h.hover("cpu-history", 0.8);
    assert_ne!(
        h.tooltip_label("cpu-hover-value"),
        first_cpu_label,
        "Hover must follow the actual sample under the cursor"
    );
    h.capture("cpu-chart-hover-next-sample");
    h.hover("memory-history", 0.5);
    let memory_hover = h.capture("memory-chart-hover");
    let memory_label = h.tooltip_label("memory-hover-value");
    assert!(
        h.observe(|v| v.metrics.containers["fixture-probe-running"]
            .samples
            .iter()
            .any(|sample| {
                sample.memory_used.is_some_and(|used| {
                    memory_label == format!("Memory: {}", crate::metrics::bytes(used))
                })
            })),
        "Memory hover must use a measured byte value, rather than the capacity percentage"
    );
    h.require_glyph_color(&memory_hover, "memory-hover-value", TEXT);
    h.hover("search", 0.5);
    // The reported usability case: a small working set against a large limit.
    for index in 0..20 {
        h.deliver(Event::Metrics {
            session: 1,
            id: "fixture-probe-running".into(),
            result: Ok(crate::metrics::Sample {
                collected: Instant::now() - Duration::from_secs((19 - index) * 2),
                cpu_percent: Some(0.12 + (index % 5) as f64 * 0.08),
                memory_used: Some(25 * 1024 * 1024 + 100 * 1024 + (index % 4) * 128 * 1024),
                memory_capacity: Some(2 * 1024 * 1024 * 1024),
                limits: Some(crate::metrics::Limits {
                    memory: Some(2 * 1024 * 1024 * 1024),
                    cpu_cores: Some(2.0),
                    cpuset: None,
                }),
                reset: index == 0,
            }),
        });
    }
    let low_usage = h.capture("metrics-low-usage");
    h.require_glyph_color(&low_usage, "memory-history", 0x74d7b1);
    h.hover("memory-history", 0.65);
    assert!(
        h.tooltip_label("memory-hover-value")
            .starts_with("Memory: 25.")
    );
    h.capture("metrics-low-usage-hover");
    h.hover("search", 0.5);
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
    h.click("toggle-compose-headless-fixture");
    h.capture("compose-group-collapsed-light");
    h.click("toggle-compose-headless-fixture");
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
    h.click("row-fixture-probe-stopped");
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-stopped".into(),
        result: Err("Late stopped sample".into()),
    });
    assert!(!h.observe(|v| v.metrics.containers.contains_key("fixture-probe-stopped")));
    h.capture("metrics-stopped");
    h.scroll_containers(-120.0);
    h.click("row-fixture-standalone");
    let unlimited = h.observe(|v| {
        v.metrics.containers["fixture-standalone"]
            .samples
            .back()
            .unwrap()
            .clone()
    });
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-standalone".into(),
        result: Ok(crate::metrics::Sample {
            collected: Instant::now(),
            ..unlimited
        }),
    });
    h.capture("metrics-no-container-limit");
    h.scroll_containers(120.0);
    h.click("row-fixture-probe-running");
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
    let generation = h.observe(|v| v.container_logs.generation);
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
        h.dock_height() < initial_dock_height - 20.0,
        "Kit divider must resize the dock through pointer input"
    );
    h.drag_dock(250.0);
    assert!(
        (h.dock_height() - 400.0).abs() < 2.0,
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
    // Toolbar clicks preserve text focus; traverse the new global controls too.
    for id in ["quick-launcher", "quick-runs", "refresh"] {
        h.key("tab");
        h.cx.update_window(h.window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find(id).focused(),
                Some(true),
                "Toolbar button {id} must own focus for activation"
            );
        })
        .unwrap();
    }
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
        window.within("popup-menu").click(8, cx);
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
    let terminal_sample = h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .samples
            .back()
            .unwrap()
            .clone()
    });
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Ok(crate::metrics::Sample {
            collected: Instant::now(),
            ..terminal_sample.clone()
        }),
    });
    let terminal_image = h.capture("terminal-rendered");
    h.require_glyph_color(&terminal_image, "cpu-metrics", TEXT);
    h.require_glyph_color(&terminal_image, "memory-metrics", TEXT);
    verify_container_row_actions(&mut h);
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
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Ok(terminal_sample),
    });
    assert_eq!(h.observe(|v| v.metrics.session), None);
    assert!(h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .error
            .is_none()
    }));
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
        worktree: Some("/fixture/project".into()),
        hint: ComposeConfig {
            project: "headless-fixture".into(),
            directory: "/fixture/project".into(),
            files: vec!["compose.yaml".into(), "compose.dev.yaml".into()],
            ..Default::default()
        },
    }]));
    let source_frame = h.capture("compose-source-worktree");
    h.require_glyph_color(&source_frame, "source-headless-fixture", 0x9eb2a8);
    assert!(
        h.cx.debug_bounds(h.window.into(), "worktree-headless-fixture")
            .unwrap()
            .is_some()
    );
    h.hover("source-headless-fixture", 0.3);
    h.settle_ui(1200);
    h.capture("compose-source-tooltip");
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
        status: "3 project containers · 2 live streams · ordered by Docker time".into(),
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
    h.click("project-log-refresh");
    let generation = h.observe(|v| v.project_logs.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::ProjectLogs { generation: current, .. } if *current == generation)));
    let restarted = crate::project_logs::Line {
        container: "fixture-probe-running".into(),
        service: "probe".into(),
        name: "headless-fixture-probe-1".into(),
        text: "2026-10-12T23:31:00Z \x1b[32mProbe restarted and ready\x1b[0m".into(),
        error: false,
    };
    let lines = std::iter::once(restarted)
        .chain((0..80).map(|index| crate::project_logs::Line {
            container: "fixture-worker-recreated".into(),
            service: "worker".into(),
            name: "headless-fixture-worker-1".into(),
            text: format!("2026-10-12T23:30:00.{index:09}Z Older worker history {index}"),
            error: false,
        }))
        .collect();
    h.deliver(Event::ProjectLogBatch { generation, lines });
    let ordered = h.capture("project-logs-restart-ordered");
    assert_eq!(
        h.observe(|v| v.project_logs.history.lines.back().unwrap().text.clone()),
        "Probe restarted and ready"
    );
    assert_eq!(
        h.observe(|v| v.project_logs.scroll.is_scrolled_to_end()),
        Some(true)
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-log-line-0")
            .unwrap()
            .is_none()
    );
    h.require_glyph_color(&ordered, "project-log-line-80", probe_color);
    h.require_glyph_color(&ordered, "project-log-line-80", crate::ansi::PALETTE[2]);
    h.click("project-service-filter");
    h.type_text("probe");
    h.key("enter");
    assert_eq!(
        h.observe(|v| v.project_logs.service.clone()).as_deref(),
        Some("probe")
    );
    let filtered = h.capture("project-logs-restart-filtered");
    h.require_glyph_color(&filtered, "project-log-line-80", probe_color);
    h.click("project-service-filter");
    h.type_text("All services");
    h.key("enter");
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
    h.click("row-primary-fixture-probe-running");
    h.click("row-restart-fixture-probe-running");
    assert!(
        h.drain_commands().is_empty(),
        "Row controls must share the project operation guard"
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

    verify_project_cleanup(&mut h);

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

    verify_images_workspace(&mut h);
    verify_volumes_workspace(&mut h);
    verify_container_row_widths(&mut h);

    let mut report = json!({
        "status": "passed", "renderer": "GPUI Kit gpui-pre WgpuHeadlessRenderer",
        "display": null, "wayland_display": null, "docker_worker_started": false,
        "logical_width": 1180, "logical_height": 820,
        "pixel_width": terminal_image.width(), "pixel_height": terminal_image.height(),
        "render_and_flow_seconds": started.elapsed().as_secs_f64(),
        "changed_pixels_after_search_lower_bound": changed_pixels, "screenshots": h.screenshots,
        "frame_timings": h.frame_timings,
        "log_performance": performance,
        "checks": ["fixture discovery", "CPU and memory rows with configured limits", "painted native area charts with CPU-percent and MiB axes", "input-driven crosshair and tooltip sample values", "low memory usage remains readable against a 2 GiB limit", "collecting unavailable stale stopped and no-limit resource summaries", "metrics session guards reject old and stopped samples", "resource summaries remain visible above the terminal", "Kit appearance Settings pointer theme switching and default restoration", "theme preference persistence in isolated file", "light theme log glyph contrast", "settings preserve selected resource", "Kit container action menu opens and dismisses", "Kit right-click Inspect menu navigation", "Kit project action menu and disabled unassociated controls", "Kit Checkbox running filter and stopped rows", "Kit menu action blocks duplicate operation", "Kit RadioGroup shell choice disabled during live session", "theme switching preserves terminal generation and input focus", "Kit service Select search and keyboard confirmation", "search pointer focus", "keyboard search", "Kit Input select all and replacement", "Kit Input caret navigation and selection", "Kit Input Unicode backspace", "empty matches", "restored selection",
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
            "project log follow pause and copy feedback", "recreated member log rendering", "return to container log routing",
            "Images sidebar and preserved container session", "Images tag and ID keyboard search", "Images virtualized 10000-row inventory", "Images stopped-container reference prevents removal", "Images review Cancel confirm duplicate guards", "Images removal success failure and refreshed references", "Images stale discovery and disconnected operation guards", "Images light-theme rendering"]
    });
    report["checks"].as_array_mut().unwrap().extend(
        [
            "Container row Start Stop Restart and resource menu requests target their own IDs",
            "Row pending and project-operation guards reject conflicting actions",
            "Other-container actions preserve selected logs and ready terminal session",
            "Row actions render in FerryDock and Light themes and at 820/1440 widths",
            "Compose source path and linked worktree tag",
            "Pointer-hover ordered Compose files and worktree tooltip",
            "Project menu cleanup review Cancel and Escape prevent execution",
            "Cleanup checkboxes send exact volume and image retention options",
            "Cleanup review invalidates on changed configuration and membership",
            "Cleanup failure remains visible; successful discovery removes project heading",
            "Compose and standalone chevrons collapse and expand rendered groups",
            "Folding preserves the detail session and survives inventory refresh",
            "Search reveals folded matches and restores folds when cleared",
            "Keyboard selection skips folded rows and handles all groups folded",
            "Bordered Compose groups render expanded and folded in FerryDock and Light themes",
            "Images Overview History Inspect tab clicks and keyboard shortcuts",
            "Images history commands UTC dates MiB sizes unavailable fields and copy feedback",
            "Images resizable history table shows aligned columns and a 53-command sample",
            "Images native divider dragging and metadata toggles preserve widths and selected rows",
            "Images history filtering context-copy horizontal wheel and first-last row navigation",
            "Images full-command dialog double-click Enter Close read-only typing and search",
            "Images history virtualizes 2000 entries and renders the Light theme",
            "Images inspection JSON syntax colors read-only typing guard and built-in search",
            "Images independent detail errors refresh retries and empty history",
            "Images selected-ID generation closed-workspace and disconnect detail guards",
            "Images column-header clicks sort ascending and descending with rendered row order",
            "Images numeric size sorting keeps unavailable last",
            "Images sorting survives search and refresh without selection or session changes",
            "Images Ctrl-click toggles multiple highlighted rows and supports empty selection",
            "Images multi-selection survives sorting filtering and refreshed snapshots",
            "Images bulk review includes hidden selections and sends exact immutable IDs",
            "Images partial bulk results retain failed selections; full success clears selection",
            "Images bulk duplicate and stale-operation guards",
            "Volumes sidebar preserves container and terminal sessions",
            "Volumes name driver and Compose-project keyboard search",
            "Volumes stopped reference guards and container navigation",
            "Volumes Cancel Escape confirmation and exact-name removal requests",
            "Volumes pending duplicate success failure and refreshed-reference guards",
            "Volumes stale generation and disconnected operation guards",
            "Volumes virtualized 10000-row inventory and Light theme",
            "Volumes sortable Name Driver Age References headers with aligned age cells",
            "Volumes age sorts by creation instant and unavailable stays last in both directions",
            "Volumes Ctrl-click selection persists through sorting search and fresh inventory",
            "Volumes bulk review includes hidden and protected selections",
            "Volumes partial results retain failures and complete success clears selection",
            "Volumes bulk duplicate stale-operation and disconnect guards",
            "Volumes prune anonymous/all-unused review Cancel Escape Enter guard and unfiltered request",
            "Volumes prune pending duplicate and container/image conflict guards",
            "Volumes prune success failure zero-space unavailable-space and cache/selection reconciliation",
            "Volumes prune workspace and disconnected stale-operation guards",
        ]
        .map(Value::from),
    );
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

fn verify_image_prune(h: &mut Harness, inventory: &[crate::images::Image]) {
    use crate::images::PruneMode;
    let selection = h.observe(|v| v.images.selection.clone());
    h.drain_commands();
    h.click("image-prune");
    assert!(h.observe(|v| v.images.prune_review));
    h.capture("images-prune-review");
    h.key("enter");
    assert!(h.observe(|v| v.images.prune_review && v.images.pruning.is_none()));
    h.click("image-prune-cancel");
    assert!(!h.observe(|v| v.images.prune_review));
    h.click("image-prune");
    h.key("escape");
    assert!(!h.observe(|v| v.images.prune_review));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::PruneImages { .. }))
    );

    // Search/selection must not become filters on this daemon-wide action.
    h.key("ctrl-f");
    h.type_text("postgres");
    h.key("escape");
    for (selector, expected, result) in [
        (
            "image-prune-untagged",
            PruneMode::Untagged,
            Ok(Some(2 * 1_048_576)),
        ),
        (
            "image-prune-unused",
            PruneMode::AllUnused,
            Err("fixture prune denied".into()),
        ),
        ("image-prune-unused", PruneMode::AllUnused, Ok(None)),
    ] {
        h.click("image-prune");
        h.click(selector);
        let operation = h.observe(|v| v.images.pruning.unwrap());
        assert!(!h.observe(|v| v.images.prune_review));
        assert_eq!(h.drain_commands().iter().filter(|command| matches!(command, Command::PruneImages { mode, operation: current } if *mode == expected && *current == operation)).count(), 1);
        h.click("image-prune");
        h.click("image-remove");
        assert!(
            h.drain_commands().is_empty(),
            "Pending prune blocks duplicate and removal requests"
        );
        h.capture("images-prune-pending");
        h.deliver(Event::ImagesPruned {
            operation: operation + 1,
            result: Ok(Some(99)),
        });
        assert_eq!(h.observe(|v| v.images.pruning), Some(operation));
        h.deliver(Event::ImagesPruned { operation, result });
        assert!(h.observe(|v| v.images.pruning.is_none()));
        assert!(
            h.drain_commands()
                .iter()
                .any(|command| matches!(command, Command::Images { .. }))
        );
        h.settle_ui(300);
        assert!(
            h.notification_count() > 0,
            "Prune completion must show feedback"
        );
        h.capture(if expected == PruneMode::Untagged {
            "images-prune-success"
        } else {
            "images-prune-feedback"
        });
        h.dismiss_image_feedback();
        let generation = h.observe(|v| v.images.generation);
        h.deliver(Event::Images {
            generation,
            result: Ok(inventory.to_vec()),
        });
        assert_eq!(h.observe(|v| v.images.selection.clone()), selection);
    }
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.drain_commands();
}

fn verify_project_cleanup(h: &mut Harness) {
    fn open(h: &mut Harness) {
        h.drain_commands();
        h.click("project-more");
        h.actions.push(json!({"input":"menu-click", "menu":"project-more", "index":8, "action":"Stop & cleanup"}));
        h.cx.update_window(h.window.into(), |_, window, cx| {
            window.within("popup-menu").click(8, cx);
        })
        .unwrap();
        h.cx.run_until_parked();
        h.settle_ui(300);
        assert!(h.observe(|v| v.projects.cleanup.is_some()));
        assert!(
            !h.drain_commands()
                .iter()
                .any(|command| matches!(command, Command::Compose { .. }))
        );
    }
    let projects = h.observe(|v| v.projects.items.clone());
    let containers = h.observe(|v| v.containers.items.clone());
    open(h);
    h.capture("compose-cleanup-review");
    h.click("compose-cleanup-cancel");
    assert!(h.observe(|v| v.projects.cleanup.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::Compose { .. }))
    );

    open(h);
    h.key("escape");
    h.settle_ui(300);
    assert!(h.observe(|v| v.projects.cleanup.is_none()));

    open(h);
    h.click("compose-cleanup-volumes");
    h.click("compose-cleanup-images");
    h.capture("compose-cleanup-keep-data");
    h.click("compose-cleanup-confirm");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action:
                    compose::Action::Cleanup {
                        volumes: false,
                        images: false,
                    },
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Reviewed cleanup options must reach worker");
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"Removing project containers\n".to_vec(),
    });
    h.capture("compose-cleanup-live");
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Err("Fixture network still in use".into()),
    });
    assert!(h.observe(|v| v.projects.items[0].config.is_some()));
    h.capture("compose-cleanup-failed");

    open(h);
    let mut changed = projects.clone();
    changed[0]
        .config
        .as_mut()
        .unwrap()
        .profiles
        .push("changed".into());
    h.deliver(Event::ComposeProjects(changed));
    assert!(h.observe(|v| v.projects.cleanup.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::Compose { .. }))
    );
    h.deliver(Event::ComposeProjects(projects.clone()));

    open(h);
    let mut changed = containers.clone();
    changed.push(container(
        "cleanup-new-member",
        "headless-fixture-new",
        "running",
        Some("headless-fixture"),
    ));
    h.deliver(Event::Snapshot(changed));
    assert!(h.observe(|v| v.projects.cleanup.is_none()));
    h.deliver(Event::Snapshot(containers.clone()));

    open(h);
    h.click("compose-cleanup-confirm");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action:
                    compose::Action::Cleanup {
                        volumes: true,
                        images: true,
                    },
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Default cleanup must remove project volumes and local images");
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Ok(()),
    });
    h.deliver(Event::ComposeProjects(vec![]));
    h.deliver(Event::Snapshot(
        containers
            .iter()
            .filter(|container| container.project.as_deref() != Some("headless-fixture"))
            .cloned()
            .collect(),
    ));
    h.capture("compose-cleanup-completed");
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-headless-fixture")
            .unwrap()
            .is_none()
    );
    h.deliver(Event::ComposeProjects(projects));
    h.deliver(Event::Snapshot(containers));
}

fn verify_container_row_widths(h: &mut Harness) {
    for (width, frame) in [
        (820.0, "container-row-actions-narrow"),
        (1440.0, "container-row-actions-wide"),
    ] {
        let mut sized = Harness::with_size(width, 820.0);
        sized.deliver(Event::Connected(Connection {
            name: "Headless fixture".into(),
            endpoint: "fixture://no-docker".into(),
        }));
        sized.deliver(Event::Snapshot(vec![container(
            "fixture-probe-running",
            "headless-fixture-probe-1",
            "running",
            Some("headless-fixture"),
        )]));
        sized.capture(frame);
        sized
            .cx
            .update_window(sized.window.into(), |_, window, cx| {
                window.render_frame(cx);
                let row = window.find("row-fixture-probe-running").bounds();
                let restart = window.find("row-restart-fixture-probe-running").bounds();
                let primary = window.find("row-primary-fixture-probe-running").bounds();
                let more = window.find("row-more-fixture-probe-running").bounds();
                assert!(restart.left() >= row.left() + px(130.0));
                assert!(restart.right() <= primary.left());
                assert!(primary.right() <= more.left());
                assert!(
                    more.right() <= row.right(),
                    "Row actions must fit the window"
                );
            })
            .unwrap();
        sized.drain_commands();
        sized.click("row-primary-fixture-probe-running");
        assert!(sized.drain_commands().iter().any(|command| matches!(command, Command::Action { id, action: Action::Stop } if id == "fixture-probe-running")));
        sized.capture(&format!("{frame}-pending"));
        h.screenshots.extend(sized.screenshots.iter().cloned());
        h.actions.extend(sized.actions.iter().cloned());
        h.frame_timings.extend(sized.frame_timings.iter().cloned());
    }
}

fn verify_container_row_actions(h: &mut Harness) {
    let dock_height = h.dock_height();
    h.drag_dock(400.0);
    let selected = h.observe(|v| v.containers.selected.clone());
    let terminal = h.observe(|v| v.terminal.view.as_ref().unwrap().entity_id());
    let generation = h.observe(|v| v.terminal.generation);
    let logs_generation = h.observe(|v| v.container_logs.generation);
    let running_only = h.observe(|v| v.containers.running_only);
    let filter = h.observe(|v| v.containers.filter.clone());
    h.click("search");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    if running_only {
        h.click("all");
    }
    h.capture("container-row-actions");
    h.drain_commands();
    h.click("row-restart-fixture-standalone");
    let commands = h.drain_commands();
    assert!(
        matches!(commands.as_slice(), [Command::Action { id, action: Action::Restart }] if id == "fixture-standalone")
    );
    h.capture("container-row-restart-pending");
    h.click("row-restart-fixture-standalone");
    h.click("row-primary-fixture-standalone");
    assert!(
        h.drain_commands().is_empty(),
        "Pending row controls must reject duplicate and conflicting actions"
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-standalone".into(),
        result: Ok(()),
    });
    h.dismiss_image_feedback();

    h.drain_commands();
    h.click("row-primary-fixture-standalone");
    assert!(
        matches!(h.drain_commands().as_slice(), [Command::Action { id, action: Action::Stop }] if id == "fixture-standalone")
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-standalone".into(),
        result: Ok(()),
    });
    h.dismiss_image_feedback();

    h.drain_commands();
    h.click("row-primary-fixture-probe-stopped");
    assert!(
        matches!(h.drain_commands().as_slice(), [Command::Action { id, action: Action::Start }] if id == "fixture-probe-stopped")
    );
    h.capture("container-row-start-pending");
    h.deliver(Event::ActionFinished {
        id: "fixture-probe-stopped".into(),
        result: Ok(()),
    });
    h.dismiss_image_feedback();

    h.click("row-more-fixture-standalone");
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()),
        selected,
        "Opening a row menu must preserve selection"
    );
    h.capture("container-row-menu");
    h.drain_commands();
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.within("popup-menu").click(8, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        matches!(h.drain_commands().as_slice(), [Command::Action { id, action: Action::Stop }] if id == "fixture-standalone")
    );
    assert_eq!(h.observe(|v| v.containers.selected.clone()), selected);
    assert!(h.observe(|v| v.tab == Tab::Terminal));
    assert_eq!(h.observe(|v| v.terminal.generation), generation);
    assert_eq!(h.observe(|v| v.container_logs.generation), logs_generation);
    assert_eq!(
        h.observe(|v| v.terminal.view.as_ref().unwrap().entity_id()),
        terminal
    );
    assert!(
        h.cx.read_entity(&h.view, |v, cx| v
            .terminal
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .ready),
        "Another container's action must keep the current shell ready"
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-standalone".into(),
        result: Ok(()),
    });
    h.dismiss_image_feedback();
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    h.capture("container-row-actions-light");
    h.click("settings");
    h.click("theme-ferrydock");
    h.click("settings");
    if running_only {
        h.click("all");
    }
    h.click("search");
    h.key("ctrl-a");
    h.type_text(&filter);
    h.key("escape");
    h.drag_dock(h.dock_height() - dock_height);
    h.drain_commands();
}

fn verify_images_workspace(h: &mut Harness) {
    use crate::images::{Image, ImageContainer};
    let connection = h.observe(|v| v.connection.clone().unwrap());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let unused = Image {
        id: format!("sha256:{}", "a".repeat(64)),
        tags: vec!["ferrydock-demo:latest".into()],
        digests: vec![],
        size: Some(128 * 1_048_576),
        created: now - 2 * 86_400,
        containers: vec![].into(),
    };
    let used = Image {
        id: format!("sha256:{}", "b".repeat(64)),
        tags: vec!["postgres:demo".into()],
        digests: vec![],
        size: Some(32 * 1_048_576),
        created: now - 3 * 86_400,
        containers: vec![ImageContainer {
            id: "fixture-probe-stopped".into(),
            name: "demo-database-stopped".into(),
            state: "exited".into(),
        }]
        .into(),
    };
    let untagged = Image {
        id: format!("sha256:{}", "c".repeat(64)),
        tags: vec![],
        size: None,
        created: 0,
        containers: vec![].into(),
        ..unused.clone()
    };
    let inventory = vec![unused.clone(), used.clone(), untagged.clone()];
    h.settle_ui(5500);
    let session = h.observe(|v| {
        (
            v.containers.selected.clone(),
            v.container_logs.generation,
            v.terminal.generation,
            v.project_logs.generation,
        )
    });
    h.drain_commands();
    h.click("0-1"); // Kit's Workspace group, Images sidebar item.
    assert!(h.observe(|v| v.images_open));
    let mut generation = h.observe(|v| v.images.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::Images { generation: current } if *current == generation)));
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.capture("images-inventory");
    verify_image_details(h, &inventory);
    verify_image_prune(h, &inventory);
    generation = h.observe(|v| v.images.generation);
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let row = window.find("image-row-aaaaaaaaaaaa").bounds();
        let size = window.find("image-size-aaaaaaaaaaaa").bounds();
        assert!(
            window.find("image-sort-name").bounds().size.width < px(200.0),
            "Image sort header must stay aligned with the image names"
        );
        assert!(
            size.origin.x > row.origin.x + row.size.width / 2.0,
            "Image sizes must align in their own column"
        );
        assert!(
            size.origin.y >= row.origin.y && size.bottom() <= row.bottom(),
            "Image metadata must stay within its rendered row"
        );
    })
    .unwrap();
    verify_image_sorting(h, &inventory, generation);
    verify_image_multiselection(h, &inventory);
    generation = h.observe(|v| v.images.generation);
    h.click("image-row-bbbbbbbbbbbb");
    h.click("image-remove");
    assert!(h.observe(|v| v.images.confirmation.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveImage { .. }))
    );
    h.capture("images-stopped-reference");
    h.key("ctrl-f");
    h.type_text("FERRYDOCK-DEMO");
    assert_eq!(h.observe(|v| v.images.visible.len()), 1);
    h.capture("images-search");
    h.key("ctrl-a");
    h.type_text("no-such-image");
    assert!(h.observe(|v| v.images.visible.is_empty()));
    h.capture("images-no-matches");
    h.key("ctrl-a");
    h.type_text("cccccccccccc");
    assert_eq!(h.observe(|v| v.images.visible.len()), 1);
    h.key("escape");
    h.key("down");
    assert_eq!(
        h.observe(|v| v.images.selected.clone()),
        Some(untagged.id.clone())
    );
    h.capture("images-untagged-unavailable-size");
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.click("image-row-aaaaaaaaaaaa");
    h.click("image-remove");
    assert_eq!(
        h.observe(|v| v.images.confirmation.clone()),
        Some(vec![unused.id.clone()])
    );
    h.capture("images-removal-review");
    assert!(
        h.cx.update_window(h.window.into(), |_, window, cx| window
            .has_active_dialog(cx))
            .unwrap()
    );
    h.click("image-cancel-remove");
    assert!(h.observe(|v| v.images.confirmation.is_none()));
    h.click("image-remove");
    h.key("escape");
    assert!(h.observe(|v| v.images.confirmation.is_none()));
    h.click("image-remove");
    // A newly attached stopped container cancels an already open review.
    let mut referenced = unused.clone();
    referenced.containers = used.containers.clone();
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![referenced, used.clone(), untagged.clone()]),
    });
    assert!(h.observe(|v| v.images.confirmation.is_none()));
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("image-remove");
    h.click("image-confirm-remove");
    let commands = h.drain_commands();
    let operation = commands
        .iter()
        .find_map(|command| match command {
            Command::RemoveImage { id, operation } if *id == unused.id => Some(*operation),
            _ => None,
        })
        .expect("Confirmed image removal must send the reviewed immutable ID");
    h.click("image-remove");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveImage { .. }))
    );
    h.capture("images-removing");
    h.deliver(Event::ImageRemoved {
        id: unused.id.clone(),
        operation,
        result: Err("Docker refused removal: image has multiple tags".into()),
    });
    h.capture("images-removal-failed");
    h.dismiss_image_feedback();
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation: generation - 1,
        result: Ok(vec![]),
    });
    assert_eq!(h.observe(|v| v.images.items.len()), 3);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("image-remove");
    h.click("image-confirm-remove");
    let operation = h
        .drain_commands()
        .iter()
        .find_map(|command| match command {
            Command::RemoveImage { id, operation } if *id == unused.id => Some(*operation),
            _ => None,
        })
        .unwrap();
    h.deliver(Event::ImageRemoved {
        id: unused.id.clone(),
        operation,
        result: Ok(()),
    });
    assert!(!h.observe(|v| v.images.items.iter().any(|image| image.id == unused.id)));
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![used.clone(), untagged.clone()]),
    });
    h.capture("images-removed");
    h.dismiss_image_feedback();
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    assert!(h.observe(|v| v.images_open && !v.settings_open));
    h.capture("images-light");
    h.click("image-prune");
    h.capture("images-prune-light");
    h.click("image-prune-cancel");
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    let many = (0..10_000)
        .map(|index| Image {
            id: format!("sha256:{index:064x}"),
            tags: vec![format!("fixture-image-{index:05}:latest")],
            ..unused.clone()
        })
        .collect();
    h.deliver(Event::Images {
        generation,
        result: Ok(many),
    });
    h.capture("images-large-inventory");
    let selection = h.observe(|v| v.images.selection.clone());
    h.drag_list_scrollbar("images-viewport", true);
    assert!(h.observe(|v| v.images.scroll.0.borrow().base_handle.offset().y < px(-1000.0)));
    assert_eq!(h.observe(|v| v.images.selection.clone()), selection);
    h.capture("images-scrollbar-dragged");
    h.drag_list_scrollbar("images-viewport", false);
    assert!(h.observe(|v| v.images.scroll.0.borrow().base_handle.offset().y > px(-100.0)));
    assert!(
        h.observe(|v| v.images.rows_rendered.get()) < 40,
        "Image list must virtualize a large inventory"
    );
    h.key("ctrl-f");
    h.type_text("fixture-image-09999");
    h.key("escape");
    h.key("down");
    assert_eq!(h.observe(|v| v.images.visible.len()), 1);
    assert_eq!(
        h.observe(|v| v.images.selected.clone()),
        Some(format!("sha256:{:064x}", 9999))
    );
    h.capture("images-large-search");
    h.click("0-0");
    assert!(!h.observe(|v| v.images_open));
    assert_eq!(
        session,
        h.observe(|v| (
            v.containers.selected.clone(),
            v.container_logs.generation,
            v.terminal.generation,
            v.project_logs.generation
        ))
    );
    h.click("0-1");
    let generation = h.observe(|v| v.images.generation);
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![unused.clone(), used.clone(), untagged.clone()]),
    });
    h.click("image-row-bbbbbbbbbbbb");
    h.click("image-container-fixture-probe-stopped");
    assert!(!h.observe(|v| v.images_open));
    assert!(!h.observe(|v| v.containers.running_only));
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()),
        Some("fixture-probe-stopped".into())
    );
    h.capture("images-reference-navigation");
    h.click("0-1");
    h.drain_commands();
    h.key("ctrl-r");
    let generation = h.observe(|v| v.images.generation);
    let commands = h.drain_commands();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::Refresh))
    );
    assert!(commands.iter().any(|command| matches!(command, Command::Images { generation: current } if *current == generation)));
    h.deliver(Event::Images {
        generation,
        result: Err("Fixture inventory failure".into()),
    });
    h.capture("images-discovery-failed");
    h.click("image-prune");
    assert!(!h.observe(|v| v.images.prune_review));
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![unused.clone(), used, untagged]),
    });
    h.click("image-row-aaaaaaaaaaaa");
    h.click("image-remove");
    h.click("image-confirm-remove");
    let operation = h.observe(|v| v.images.busy[&unused.id]);
    h.click_tab("image-tabs", 1);
    let detail_generation = h.observe(|v| v.images.details.generation);
    h.deliver(Event::Disconnected("Fixture Docker disconnect".into()));
    h.deliver(Event::ImageHistory {
        id: unused.id.clone(),
        generation: detail_generation,
        result: Ok(vec![]),
    });
    h.deliver(Event::ImageInspection {
        id: unused.id.clone(),
        generation: detail_generation,
        result: Ok("stale disconnected inspection".into()),
    });
    assert!(
        h.observe(|v| v.images.details.history.is_none() && v.images.details.inspection.is_none())
    );
    h.capture("images-history-disconnected");
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.deliver(Event::ImageRemoved {
        id: unused.id,
        operation,
        result: Ok(()),
    });
    assert!(!h.observe(|v| v.images.loaded));
    assert!(h.observe(|v| v.images.busy.is_empty()));
    assert!(h.observe(|v| {
        v.images
            .items
            .iter()
            .any(|image| image.id == format!("sha256:{}", "a".repeat(64)))
    }));
    h.capture("images-disconnected");
    h.deliver(Event::Connected(connection.clone()));
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::ImageDetails { id, .. } if id == &format!("sha256:{}", "a".repeat(64)))), "Reconnect reloads active image details");
    h.key("ctrl-1");
    h.click("image-prune");
    assert!(h.observe(|v| v.images.prune_review));
    h.deliver(Event::Disconnected(
        "Fixture disconnected during prune review".into(),
    ));
    assert!(!h.observe(|v| v.images.prune_review));
    assert!(
        !h.cx
            .update_window(h.window.into(), |_, window, cx| window
                .has_active_dialog(cx))
            .unwrap()
    );
    h.deliver(Event::Connected(connection));
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory),
    });
    h.click("image-prune");
    h.click("image-prune-unused");
    let operation = h.observe(|v| v.images.pruning.unwrap());
    h.deliver(Event::Disconnected(
        "Fixture disconnected during pruning".into(),
    ));
    h.drain_commands();
    h.deliver(Event::ImagesPruned {
        operation,
        result: Ok(Some(1_048_576)),
    });
    assert!(h.observe(|v| v.images.pruning.is_none() && !v.images.loaded));
    assert!(
        h.drain_commands().is_empty(),
        "Disconnected prune completion is ignored"
    );
    h.capture("images-prune-disconnected");
}

fn verify_image_details(h: &mut Harness, inventory: &[crate::images::Image]) {
    use super::image_details::ImageTab;
    use bollard::models::ImageHistoryResponseItem;
    let id = inventory[0].id.clone();
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Overview));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::ImageDetails { .. }))
    );
    h.click_tab("image-tabs", 1);
    let generation = h.observe(|v| v.images.details.generation);
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::ImageDetails { id: current, generation: active } if current == &id && *active == generation)));
    h.capture("images-history-loading");
    let rows = vec![
        ImageHistoryResponseItem {
            id: id.clone(),
            created: 1_792_000_000,
            created_by: "CMD [\"/bin/sh\"]".into(),
            size: 0,
            comment: "buildkit.dockerfile.v0".into(),
            ..Default::default()
        },
        ImageHistoryResponseItem {
            id: "<missing>".into(),
            created: 1_791_900_000,
            created_by: format!(
                "RUN /bin/sh -c {}",
                "echo complete-build-command; ".repeat(20)
            ),
            size: 3 * 1_048_576,
            ..Default::default()
        },
        ImageHistoryResponseItem {
            id: "<missing>".into(),
            created: 0,
            size: -1,
            ..Default::default()
        },
    ];
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation,
        result: Ok(rows.clone()),
    });
    h.deliver(Event::ImageInspection {
        id: id.clone(),
        generation,
        result: Err("Fixture inspection unavailable".into()),
    });
    h.capture("images-history");
    assert_eq!(
        h.observe(|v| v
            .images
            .details
            .history
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .len()),
        3
    );
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.delegate().columns, [true; 3]);
        assert_eq!(table.delegate().active_columns(), vec![0, 1, 2, 3, 4]);
        assert_eq!(table.delegate().visible.len(), 3);
    });
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let command = window.find("history-cell-1-1").bounds();
        let size = window.find("history-cell-1-2").bounds();
        assert!(
            command.right() <= size.origin.x,
            "Long commands must stay inside their column"
        );
    })
    .unwrap();
    h.history_click_line(0, MouseButton::Left);
    let width = h.cx.read_entity(&h.view, |view, cx| {
        view.images
            .details
            .history_view
            .table
            .read(cx)
            .delegate()
            .widths[1]
    });
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let header = window
            .within("image-history-table")
            .find(("col-header", 1_usize))
            .bounds();
        let from = gpui::point(header.right() - px(1.0), header.center().y);
        window.drag(from, from + gpui::point(px(200.0), px(0.0)), cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.capture("images-history-table-resized");
    h.cx.read_entity(&h.view, |view, cx| {
        assert!(
            view.images
                .details
                .history_view
                .table
                .read(cx)
                .delegate()
                .widths[1]
                > width + px(40.0),
            "Pointer dragging must resize the command column"
        )
    });
    for column in 0..3 {
        h.click("image-history-columns");
        h.cx.update_window(h.window.into(), |_, window, cx| {
            // Register the newly opened popup after dialog/focus transitions.
            window.render_frame(cx);
            window.within("popup-menu").click(column, cx)
        })
        .unwrap();
        h.cx.run_until_parked();
    }
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.delegate().columns, [false; 3]);
        assert_eq!(table.selected_row(), Some(0));
        assert!(table.delegate().widths[1] > width + px(40.0));
    });
    h.capture("images-history-command-only");
    h.key("ctrl-f");
    h.type_text("complete-build-command");
    h.cx.read_entity(&h.view, |view, cx| {
        assert_eq!(
            view.images
                .details
                .history_view
                .table
                .read(cx)
                .delegate()
                .visible,
            vec![1]
        )
    });
    h.capture("images-history-search");
    h.key("escape");
    h.history_click_line(1, MouseButton::Right);
    h.capture("images-history-context-menu");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.within("popup-menu").click(0, cx)
    })
    .unwrap();
    h.cx.run_until_parked();
    h.cx.update_window(h.window.into(), |_, _, cx| {
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(rows[1].created_by.clone())
        )
    })
    .unwrap();
    h.history_click_line(1, MouseButton::Left);
    h.key("ctrl-c");
    h.key("enter");
    assert!(h.observe(|v| v.images.details.history_view.command_open));
    h.capture("images-history-full-command");
    h.key("ctrl-a");
    h.type_text("must-not-edit-command");
    h.cx.read_entity(&h.view, |view, cx| {
        assert_eq!(
            view.images
                .details
                .history_view
                .command_editor
                .read(cx)
                .value(),
            rows[1].created_by
        )
    });
    h.key("ctrl-f");
    h.type_text("complete-build-command");
    h.cx.read_entity(&h.view, |view, cx| {
        let search = view
            .images
            .details
            .history_view
            .command_editor
            .read(cx)
            .search_session();
        assert!(search.open && !search.replace_mode);
        assert_eq!(search.matcher.len(), 20);
    });
    h.capture("images-history-full-command-search");
    h.key("escape");
    h.click("history-command-close");
    assert!(!h.observe(|v| v.images.details.history_view.command_open));
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let position = window.find("history-row-1").bounds().center();
        window.dispatch_event(
            MouseDownEvent {
                position,
                button: MouseButton::Left,
                modifiers: Modifiers::none(),
                click_count: 2,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseUpEvent {
                position,
                button: MouseButton::Left,
                modifiers: Modifiers::none(),
                click_count: 2,
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        h.observe(|v| v.images.details.history_view.command_open),
        "Double click must open the full command"
    );
    h.click("history-command-close");
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    for column in 0..3 {
        h.click("image-history-columns");
        h.cx.update_window(h.window.into(), |_, window, cx| {
            // Register the newly opened popup after dialog/focus transitions.
            window.render_frame(cx);
            window.within("popup-menu").click(column, cx)
        })
        .unwrap();
        h.cx.run_until_parked();
    }
    h.capture("images-history-columns");
    let left =
        h.cx.debug_bounds(h.window.into(), "history-header-command")
            .unwrap()
            .unwrap()
            .origin
            .x;
    h.history_scroll(-300.0, 0.0);
    h.capture("images-history-horizontal-scroll");
    let moved =
        h.cx.debug_bounds(h.window.into(), "history-header-command")
            .unwrap()
            .unwrap()
            .origin
            .x;
    assert!(moved < left - px(30.0));
    h.history_scroll(10_000.0, 0.0);
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let header = window
            .within("image-history-table")
            .find(("col-header", 1_usize))
            .bounds();
        let from = gpui::point(header.right() - px(1.0), header.center().y);
        window.drag(from, from - gpui::point(px(200.0), px(0.0)), cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    let realistic = (0..53)
        .map(|index| ImageHistoryResponseItem {
            created_by: match index % 3 {
                0 => "USER www-data".into(),
                1 => format!("RUN /bin/sh -c echo build-step-{index}"),
                _ => "WORKDIR /var/www/html".into(),
            },
            ..rows[index % rows.len()].clone()
        })
        .collect();
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation,
        result: Ok(realistic),
    });
    h.capture("images-history-53-commands");
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.delegate().visible.len(), 53);
        assert!(table.visible_range().rows().len() >= 5);
    });
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation,
        result: Ok(rows.clone()),
    });
    h.click_tab("image-tabs", 2);
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Inspect));
    assert!(
        h.drain_commands().is_empty(),
        "Tab changes reuse selected-image results"
    );
    h.capture("images-inspect-failed");
    h.key("ctrl-f");
    h.type_text("ferry");
    h.deliver(Event::ImageInspection {
        id: id.clone(),
        generation,
        result: Err("Fixture error while searching inventory".into()),
    });
    h.type_text("dock");
    assert_eq!(
        h.observe(|v| v.images.filter.clone()),
        "ferrydock",
        "An async inspection error must preserve inventory-search focus"
    );
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    // Refresh retries both endpoints without changing the selected tab.
    h.key("ctrl-r");
    let inventory_generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation: inventory_generation,
        result: Ok(inventory.to_vec()),
    });
    let active = h.observe(|v| v.images.details.generation);
    assert_ne!(active, generation);
    let inspection = serde_json::to_string_pretty(&json!({
        "Id": id, "Architecture": "amd64", "Config": { "Cmd": ["/bin/sh"], "Env": ["IMAGE=fixture-only"] },
        "RootFS": { "Type": "layers", "Layers": ["sha256:fixture-layer"] }
    })).unwrap();
    h.deliver(Event::ImageInspection {
        id: id.clone(),
        generation: active,
        result: Ok(inspection.clone()),
    });
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation: active,
        result: Ok(rows.clone()),
    });
    // Clicking the tab must focus the Kit editor, even after an async load.
    h.click_tab("image-tabs", 2);
    let painted = h.capture("images-inspect-json");
    h.cx.read_entity(&h.view, |view, cx| {
        assert_eq!(view.images.details.editor.read(cx).value(), inspection)
    });
    let view = h.view.clone();
    let body =
        h.cx.update_window(h.window.into(), |_, _, cx| {
            view.read(cx)
                .images
                .details
                .editor
                .read(cx)
                .text_bounds()
                .unwrap()
        })
        .unwrap();
    assert!(
        painted
            .enumerate_pixels()
            .filter(|(x, y, pixel)| {
                *x >= (f32::from(body.origin.x) * 2.0) as u32
                    && *x < (f32::from(body.right()) * 2.0) as u32
                    && *y >= (f32::from(body.origin.y) * 2.0) as u32
                    && *y < (f32::from(body.bottom()) * 2.0) as u32
                    && pixel.0[..3].iter().max().unwrap() - pixel.0[..3].iter().min().unwrap() > 50
            })
            .count()
            > 200,
        "Image inspection must paint JSON syntax colors"
    );
    h.key("ctrl-a");
    h.type_text("must-not-edit-image-inspection");
    h.cx.read_entity(&h.view, |view, cx| {
        assert_eq!(view.images.details.editor.read(cx).value(), inspection)
    });
    h.key("ctrl-f");
    h.type_text("Architecture");
    h.cx.read_entity(&h.view, |view, cx| {
        let search = view.images.details.editor.read(cx).search_session();
        assert!(search.open && !search.replace_mode);
        assert_eq!(search.query, "Architecture");
        assert_eq!(search.matcher.len(), 1);
    });
    h.capture("images-inspect-search");
    h.key("escape");
    h.click_tab("image-tabs", 1);
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation: active,
        result: Ok(vec![rows[2].clone()]),
    });
    h.capture("images-history-unavailable-values");
    let many = (0..2_000)
        .map(|index| ImageHistoryResponseItem {
            created_by: format!("RUN echo history-{index}"),
            ..rows[0].clone()
        })
        .collect();
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation: active,
        result: Ok(many),
    });
    h.capture("images-history-large");
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.delegate().visible.len(), 2_000);
        assert!(table.visible_range().rows().len() < 40);
    });
    h.history_click_line(0, MouseButton::Left);
    h.key("end");
    h.capture("images-history-tail");
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.selected_row(), Some(1_999));
        assert_eq!(table.visible_range().rows().end, 2_000);
    });
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    h.capture("images-history-light");
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    h.click("image-row-bbbbbbbbbbbb");
    let second = h.observe(|v| v.images.details.generation);
    assert!(
        h.observe(|v| v.images.details.history.is_none() && v.images.details.inspection.is_none())
    );
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation: active,
        result: Ok(rows.clone()),
    });
    h.deliver(Event::ImageInspection {
        id: id.clone(),
        generation: second,
        result: Ok(inspection.clone()),
    });
    assert!(
        h.observe(|v| v.images.details.history.is_none() && v.images.details.inspection.is_none()),
        "Old generation and wrong image ID must be ignored"
    );
    h.deliver(Event::ImageHistory {
        id: inventory[1].id.clone(),
        generation: second,
        result: Ok(vec![]),
    });
    h.capture("images-history-empty");
    h.deliver(Event::ImageHistory {
        id: inventory[1].id.clone(),
        generation: second,
        result: Err("Fixture history unavailable".into()),
    });
    h.capture("images-history-failed");
    h.click("0-0");
    h.deliver(Event::ImageInspection {
        id: inventory[1].id.clone(),
        generation: second,
        result: Ok(inspection),
    });
    assert!(
        h.observe(|v| v.images.details.inspection.is_none()),
        "Closed workspace rejects detail completions"
    );
    h.click("0-1");
    let inventory_generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation: inventory_generation,
        result: Ok(inventory.to_vec()),
    });
    h.click("image-row-aaaaaaaaaaaa");
    h.key("ctrl-3");
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Inspect));
    h.key("ctrl-2");
    assert!(h.observe(|v| v.images.details.tab == ImageTab::History));
    h.key("ctrl-1");
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Overview));
    h.click("0-0");
    h.click("0-1");
    let inventory_generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation: inventory_generation,
        result: Ok(inventory.to_vec()),
    });
    h.drain_commands();
}

fn verify_image_sorting(h: &mut Harness, inventory: &[crate::images::Image], generation: u64) {
    let selected = h.observe(|v| v.images.selected.clone());
    let assert_order = |h: &Harness, expected: &[usize]| {
        let ids = h.observe(|v| {
            v.images
                .visible
                .iter()
                .map(|index| v.images.items[*index].id.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            ids,
            expected
                .iter()
                .map(|index| inventory[*index].id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            h.observe(|v| v.images.selected.clone()),
            selected,
            "Sorting must preserve the selected image ID"
        );
    };
    h.drain_commands();
    h.click("image-sort-size");
    assert_order(h, &[1, 0, 2]); // 32 MiB before 128 MiB; unavailable stays last.
    h.capture("images-size-ascending");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let small = window.find("image-row-bbbbbbbbbbbb").bounds();
        let large = window.find("image-row-aaaaaaaaaaaa").bounds();
        let missing = window.find("image-row-cccccccccccc").bounds();
        assert!(
            small.origin.y < large.origin.y && large.origin.y < missing.origin.y,
            "Sorted image order must reach the rendered rows"
        );
    })
    .unwrap();
    h.click("image-sort-size");
    assert_order(h, &[0, 1, 2]);
    h.capture("images-size-descending");
    let mut refreshed = inventory.to_vec();
    refreshed.reverse();
    h.deliver(Event::Images {
        generation,
        result: Ok(refreshed),
    });
    assert_order(h, &[0, 1, 2]);
    h.click("image-sort-age");
    assert_order(h, &[0, 1, 2]);
    h.capture("images-age-youngest-first");
    h.click("image-sort-age");
    assert_order(h, &[1, 0, 2]);
    h.capture("images-age-oldest-first");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let created = window.find("image-age-aaaaaaaaaaaa").bounds();
        let size = window.find("image-size-aaaaaaaaaaaa").bounds();
        assert!(
            created.origin.x < size.origin.x
                && created.origin.y >= window.find("image-row-aaaaaaaaaaaa").bounds().origin.y,
            "Image ages must render in their own aligned column"
        );
    })
    .unwrap();
    h.click("image-sort-age");
    assert_order(h, &[0, 1, 2]);
    h.key("ctrl-f");
    h.type_text("postgres");
    assert_order(h, &[1]);
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    assert_order(h, &[0, 1, 2]);
    h.click("image-sort-references");
    assert_order(h, &[0, 2, 1]);
    h.click("image-sort-references");
    assert_order(h, &[1, 0, 2]);
    h.capture("images-references-descending");
    h.click("image-sort-name");
    assert_order(h, &[0, 1, 2]);
    h.click("image-sort-name");
    assert_order(h, &[2, 1, 0]);
    h.capture("images-name-descending");
    h.click("image-sort-name");
    assert_order(h, &[0, 1, 2]);
    assert!(
        h.drain_commands().is_empty(),
        "Sorting must not restart worker sessions"
    );
}

fn verify_image_multiselection(h: &mut Harness, inventory: &[crate::images::Image]) {
    use crate::images::RemovalResult;
    let ids = inventory
        .iter()
        .map(|image| image.id.clone())
        .collect::<Vec<_>>();
    h.click("image-row-aaaaaaaaaaaa");
    h.ctrl_click("image-row-cccccccccccc");
    assert_eq!(
        h.observe(|v| v.images.selection.iter().cloned().collect::<Vec<_>>()),
        vec![ids[0].clone(), ids[2].clone()]
    );
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("image-row-aaaaaaaaaaaa").selected(), Some(true));
        assert_eq!(window.find("image-row-cccccccccccc").selected(), Some(true));
    })
    .unwrap();
    h.ctrl_click("image-row-cccccccccccc");
    h.ctrl_click("image-row-aaaaaaaaaaaa");
    assert!(h.observe(|v| v.images.selection.is_empty() && v.images.selected.is_none()));
    h.drain_commands();
    h.click("image-remove");
    assert!(h.drain_commands().is_empty());
    h.click("image-row-aaaaaaaaaaaa");
    h.ctrl_click("image-row-cccccccccccc");
    h.capture("images-multiple-selected");
    h.click("image-sort-size");
    assert_eq!(h.observe(|v| v.images.selection.len()), 2);
    h.key("ctrl-f");
    h.type_text("ferrydock-demo");
    h.key("escape");
    assert_eq!(h.observe(|v| v.images.visible.len()), 1);
    let generation = h.observe(|v| v.images.generation);
    let mut refreshed = inventory.to_vec();
    refreshed.reverse();
    h.deliver(Event::Images {
        generation,
        result: Ok(refreshed),
    });
    assert_eq!(h.observe(|v| v.images.selection.len()), 2);
    h.capture("images-multiple-hidden-selection");
    h.click("image-remove");
    assert_eq!(
        h.observe(|v| v.images.confirmation.clone()),
        Some(vec![ids[0].clone(), ids[2].clone()])
    );
    h.capture("images-multiple-hidden-review");
    h.click("image-cancel-remove");
    assert!(h.drain_commands().is_empty());
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.click("image-sort-name");
    h.ctrl_click("image-row-bbbbbbbbbbbb");
    assert_eq!(h.observe(|v| v.images.selection.len()), 3);
    h.click("image-remove");
    h.capture("images-multiple-removal-review");
    h.click("image-confirm-remove");
    let operation = h.drain_commands().iter().find_map(|command| match command {
        Command::RemoveImages { ids: selected, operation } if *selected == ids => Some(*operation), _ => None,
    }).expect("Bulk deletion must send exactly the reviewed immutable IDs, including hidden selections");
    assert_eq!(h.observe(|v| v.images.busy.len()), 3);
    h.click("image-remove");
    assert!(h.drain_commands().is_empty());
    h.capture("images-multiple-removing");
    h.deliver(Event::ImagesRemoved {
        operation: operation - 1,
        results: vec![RemovalResult {
            id: ids[1].clone(),
            result: Ok(()),
        }],
    });
    assert_eq!(h.observe(|v| v.images.busy.len()), 3);
    h.deliver(Event::ImagesRemoved {
        operation,
        results: vec![
            RemovalResult {
                id: ids[0].clone(),
                result: Ok(()),
            },
            RemovalResult {
                id: ids[1].clone(),
                result: Err("Image is used by a stopped container".into()),
            },
            RemovalResult {
                id: ids[2].clone(),
                result: Err("Docker refused removal: image has multiple tags".into()),
            },
        ],
    });
    assert!(h.observe(|v| v.images.busy.is_empty()));
    assert!(!h.observe(|v| v.images.items.iter().any(|image| image.id == ids[0])));
    assert_eq!(
        h.observe(|v| v.images.selection.iter().cloned().collect::<Vec<_>>()),
        vec![ids[1].clone(), ids[2].clone()]
    );
    h.capture("images-multiple-partial-results");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("image-removal-results").is_none());
    })
    .unwrap();
    h.dismiss_image_feedback();
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let detail = window.find("image-detail").bounds();
        assert!(
            detail.size.height >= px(260.0) && detail.bottom() <= px(820.0),
            "Deletion feedback must leave the detail panel inside the window"
        );
    })
    .unwrap();
    let generation = h.observe(|v| v.images.generation);
    let mut available = inventory[1].clone();
    available.containers = std::sync::Arc::from([]);
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![available, inventory[2].clone()]),
    });
    assert_eq!(h.observe(|v| v.images.selection.len()), 2);
    h.click("image-remove");
    assert!(
        h.observe(|v| v.images.confirmation.is_some()),
        "Failed images must remain reviewable after inventory refresh"
    );
    h.click("image-confirm-remove");
    let operation = h
        .drain_commands()
        .iter()
        .find_map(|command| match command {
            Command::RemoveImages { operation, .. } => Some(*operation),
            _ => None,
        })
        .unwrap();
    h.deliver(Event::ImagesRemoved {
        operation,
        results: vec![
            RemovalResult {
                id: ids[1].clone(),
                result: Ok(()),
            },
            RemovalResult {
                id: ids[2].clone(),
                result: Ok(()),
            },
        ],
    });
    assert!(h.observe(|v| v.images.selection.is_empty()
        && v.images.selected.is_none()
        && v.images.items.is_empty()));
    h.capture("images-multiple-removed");
    h.dismiss_image_feedback();
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.to_vec()),
    });
    h.click("image-row-aaaaaaaaaaaa");
    assert_eq!(h.observe(|v| v.images.selection.len()), 1);
    h.drain_commands();
}

fn verify_volumes_workspace(h: &mut Harness) {
    use crate::volumes::{Volume, VolumeContainer};
    h.deliver(Event::Connected(crate::docker::Connection {
        name: "headless-fixture".into(),
        endpoint: "unix:///headless-fixture.sock".into(),
    }));
    h.settle_ui(5500);
    h.click("0-0");
    h.click("row-fixture-probe-running");
    let unused = Volume {
        name: "demo-cache".into(),
        driver: "local".into(),
        mountpoint: "/var/lib/docker/volumes/demo-cache/_data".into(),
        created: Some("2026-10-13T10:00:00-04:00".into()),
        scope: Some("local".into()),
        labels: [("com.docker.compose.project".into(), "sample-project".into())].into(),
        options: Default::default(),
        size: None,
        containers: vec![].into(),
    };
    let used = Volume {
        name: "demo-database".into(),
        containers: vec![VolumeContainer {
            id: "fixture-probe-stopped".into(),
            name: "demo-database-stopped".into(),
            state: "exited".into(),
        }]
        .into(),
        ..unused.clone()
    };
    let remote = Volume {
        name: "remote-data".into(),
        driver: "nfs".into(),
        labels: Default::default(),
        options: [("type".into(), "nfs".into())].into(),
        ..unused.clone()
    };
    let inventory = vec![unused.clone(), used.clone(), remote.clone()];
    let session = h.observe(|v| {
        (
            v.containers.selected.clone(),
            v.container_logs.generation,
            v.terminal.generation,
            v.project_logs.generation,
        )
    });
    h.drain_commands();
    h.click("0-2"); // Workspace -> Volumes.
    assert!(h.observe(|v| v.volumes_open && !v.images_open));
    let mut generation = h.observe(|v| v.volumes.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::Volumes { generation: current } if *current == generation)));
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.capture("volumes-inventory");
    verify_volume_sizes(h, &inventory);
    verify_volume_sorting_and_selection(h, &inventory);
    generation = h.observe(|v| v.volumes.generation);
    h.click("volume-row-demo-database");
    h.click("volume-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveVolume { .. }))
    );
    h.capture("volumes-stopped-reference");
    for (query, count) in [
        ("DEMO-CACHE", 1),
        ("NFS", 1),
        ("SAMPLE-PROJECT", 2),
        ("no-such-volume", 0),
    ] {
        h.key("ctrl-f");
        h.key("ctrl-a");
        h.type_text(query);
        assert_eq!(h.observe(|v| v.volumes.visible.len()), count);
    }
    h.capture("volumes-no-matches");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.click("volume-row-demo-cache");
    h.click("volume-remove");
    assert_eq!(
        h.observe(|v| v.volumes.confirmation.clone()),
        Some(vec![unused.name.clone()])
    );
    h.capture("volumes-removal-review");
    h.click("volume-cancel-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.click("volume-remove");
    h.key("escape");
    h.settle_ui(300);
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveVolume { .. }))
    );
    // A stopped reference appearing during review invalidates the open dialog.
    h.click("volume-remove");
    let mut now_used = unused.clone();
    now_used.containers = used.containers.clone();
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![now_used, used.clone(), remote.clone()]),
    });
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h
        .drain_commands()
        .iter()
        .find_map(|command| match command {
            Command::RemoveVolume { name, operation } if name == &unused.name => Some(*operation),
            _ => None,
        })
        .unwrap();
    h.click("volume-remove");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveVolume { .. }))
    );
    h.capture("volumes-removing");
    h.click("0-0");
    h.drain_commands();
    h.click("primary-action");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::Action { .. }))
    );
    h.click("0-2");
    h.deliver(Event::VolumeRemoved {
        name: unused.name.clone(),
        operation: operation + 1,
        result: Ok(()),
    });
    assert!(h.observe(|v| v.volumes.busy.is_some()));
    h.deliver(Event::VolumeRemoved {
        name: unused.name.clone(),
        operation,
        result: Err("Fixture volume conflict".into()),
    });
    assert!(h.observe(|v| v.volumes.busy.is_none()));
    assert_eq!(
        h.observe(|v| v.volumes.selected.clone()),
        Some(unused.name.clone())
    );
    h.capture("volumes-removal-failed");
    h.dismiss_image_feedback();
    generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h.observe(|v| v.volumes.busy.as_ref().unwrap().1);
    let old_generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::VolumeRemoved {
        name: unused.name.clone(),
        operation,
        result: Ok(()),
    });
    h.deliver(Event::Volumes {
        generation: old_generation,
        result: Ok(inventory.clone()),
    });
    assert!(!h.observe(|v| {
        v.volumes
            .items
            .iter()
            .any(|volume| volume.name == unused.name)
    }));
    generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![used.clone(), remote.clone()]),
    });
    h.capture("volumes-removed");
    h.dismiss_image_feedback();
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    assert!(h.observe(|v| v.volumes_open && !v.settings_open));
    h.capture("volumes-light");
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    let many = (0..10_000)
        .map(|index| Volume {
            name: format!("fixture-volume-{index:05}"),
            ..unused.clone()
        })
        .collect();
    h.deliver(Event::Volumes {
        generation,
        result: Ok(many),
    });
    h.capture("volumes-large-inventory");
    let selection = h.observe(|v| v.volumes.selection.clone());
    h.drag_list_scrollbar("volumes-viewport", true);
    assert!(h.observe(|v| v.volumes.scroll.0.borrow().base_handle.offset().y < px(-1000.0)));
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selection);
    h.capture("volumes-scrollbar-dragged");
    h.drag_list_scrollbar("volumes-viewport", false);
    assert!(h.observe(|v| v.volumes.scroll.0.borrow().base_handle.offset().y > px(-100.0)));
    assert!(h.observe(|v| v.volumes.rows_rendered.get()) < 40);
    h.key("ctrl-f");
    h.type_text("fixture-volume-09999");
    h.key("escape");
    h.key("down");
    assert_eq!(h.observe(|v| v.volumes.visible.len()), 1);
    assert_eq!(
        h.observe(|v| v.volumes.selected.clone()),
        Some("fixture-volume-09999".into())
    );
    h.capture("volumes-large-search");
    h.click("0-0");
    assert!(!h.observe(|v| v.volumes_open));
    assert_eq!(
        session,
        h.observe(|v| (
            v.containers.selected.clone(),
            v.container_logs.generation,
            v.terminal.generation,
            v.project_logs.generation
        ))
    );
    assert!(
        h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::CloseVolumes))
    );
    h.click("0-2");
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("volume-row-demo-database");
    h.click("volume-container-fixture-probe-stopped");
    assert!(!h.observe(|v| v.volumes_open));
    assert!(!h.observe(|v| v.containers.running_only));
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()),
        Some("fixture-probe-stopped".into())
    );
    h.capture("volumes-reference-navigation");
    h.click("0-2");
    h.drain_commands();
    h.key("ctrl-r");
    generation = h.observe(|v| v.volumes.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::Volumes { generation: current } if *current == generation)));
    h.deliver(Event::Volumes {
        generation,
        result: Err("Fixture volume inventory failure".into()),
    });
    h.capture("volumes-discovery-failed");
    h.click("volume-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("volume-gather-sizes");
    let request = h.observe(|v| v.volumes.gathering.unwrap());
    let measurement = crate::volumes::VolumeSize {
        name: unused.name.clone(),
        driver: unused.driver.clone(),
        mountpoint: unused.mountpoint.clone(),
        created: unused.created.clone(),
        size: Some(3 * 1_048_576),
    };
    h.deliver(Event::VolumeSizes {
        request,
        result: Ok(vec![measurement.clone()]),
    });
    h.click("volume-gather-sizes");
    let interrupted_request = h.observe(|v| v.volumes.gathering.unwrap());
    h.click("volume-row-demo-cache");
    h.ctrl_click("volume-row-remote-data");
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h.observe(|v| v.volumes.busy.as_ref().unwrap().1);
    h.deliver(Event::Disconnected("Fixture Docker disconnect".into()));
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory),
    });
    h.deliver(Event::VolumesRemoved {
        operation,
        results: vec![
            crate::volumes::RemovalResult {
                name: unused.name.clone(),
                result: Ok(()),
            },
            crate::volumes::RemovalResult {
                name: remote.name.clone(),
                result: Ok(()),
            },
        ],
    });
    assert!(!h.observe(|v| v.volumes.loaded));
    assert!(h.observe(|v| v.volumes.busy.is_none()));
    assert!(h.observe(|v| {
        v.volumes
            .items
            .iter()
            .any(|volume| volume.name == unused.name)
    }));
    h.deliver(Event::VolumeSizes {
        request: interrupted_request,
        result: Ok(vec![]),
    });
    assert!(h.observe(|v| v.volumes.gathering.is_none()));
    assert_eq!(
        h.observe(|v| v
            .volumes
            .items
            .iter()
            .find(|volume| volume.name == unused.name)
            .unwrap()
            .size),
        measurement.size
    );
    h.drain_commands();
    h.click("volume-gather-sizes");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::GatherVolumeSizes { .. }))
    );
    h.deliver(Event::Connected(Connection {
        name: "headless-fixture".into(),
        endpoint: "unix:///headless-fixture.sock".into(),
    }));
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![unused.clone(), used.clone(), remote.clone()]),
    });
    assert_eq!(h.observe(|v| v.volumes.items[0].size), measurement.size);
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::GatherVolumeSizes { .. }))
    );
    h.capture("volumes-sizes-retained-after-reconnect");
    h.deliver(Event::Connected(Connection {
        name: "other-fixture".into(),
        endpoint: "unix:///other-fixture.sock".into(),
    }));
    assert!(h.observe(|v| v.volumes.items.iter().all(|volume| volume.size.is_none())));
    h.deliver(Event::Disconnected("Fixture Docker disconnect".into()));
    h.capture("volumes-disconnected");
    verify_volume_pruning(h, &[unused, used, remote]);
}

fn verify_volume_pruning(h: &mut Harness, original: &[crate::volumes::Volume]) {
    use crate::volumes::{PruneMode, PruneResult, Volume, VolumeSize};
    let connection = Connection {
        name: "headless-fixture".into(),
        endpoint: "unix:///headless-fixture.sock".into(),
    };
    h.deliver(Event::Connected(connection.clone()));
    let mut inventory = original.to_vec();
    inventory.push(Volume {
        name: "hidden-unused".into(),
        ..original[0].clone()
    });
    let deliver = |h: &mut Harness, inventory: &[Volume]| {
        let generation = h.observe(|v| v.volumes.generation);
        h.deliver(Event::Volumes {
            generation,
            result: Ok(inventory.to_vec()),
        });
    };
    deliver(h, &inventory);
    h.click("volume-row-demo-cache");
    h.drain_commands();
    h.click("volume-gather-sizes");
    let request = h.observe(|v| v.volumes.gathering.unwrap());
    let measurements = inventory
        .iter()
        .enumerate()
        .map(|(index, volume)| VolumeSize {
            name: volume.name.clone(),
            driver: volume.driver.clone(),
            mountpoint: volume.mountpoint.clone(),
            created: volume.created.clone(),
            size: Some(index as u64 * 1_048_576),
        })
        .collect();
    h.deliver(Event::VolumeSizes {
        request,
        result: Ok(measurements),
    });
    h.ctrl_click("volume-row-demo-cache");
    assert!(h.observe(|v| v.volumes.selection.is_empty()));
    h.key("ctrl-f");
    h.type_text("demo-database");
    h.key("escape");
    assert_eq!(h.observe(|v| v.volumes.visible.len()), 1);
    h.drain_commands();
    h.click("volume-prune");
    assert!(h.observe(|v| v.volumes.prune_review));
    h.capture("volumes-prune-review-all-unused");
    h.key("enter");
    assert!(h.observe(|v| v.volumes.prune_review && v.volumes.pruning.is_none()));
    h.click("volume-prune-cancel");
    assert!(!h.observe(|v| v.volumes.prune_review));
    h.click("volume-prune");
    h.key("escape");
    assert!(!h.observe(|v| v.volumes.prune_review));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::PruneVolumes { .. }))
    );
    // A conflicting worker action disables prune, even with a loaded inventory.
    h.deliver(Event::ActionStarted {
        id: "fixture-probe-running".into(),
        action: Action::Stop,
    });
    h.click("volume-prune");
    assert!(!h.observe(|v| v.volumes.prune_review));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::PruneVolumes { .. }))
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-probe-running".into(),
        result: Ok(()),
    });
    deliver(h, &inventory);
    h.dismiss_image_feedback();
    for (index, result) in [
        Err("Fixture Docker prune failure".into()),
        Ok(PruneResult {
            deleted: vec![original[0].name.clone(), "hidden-unused".into()],
            reclaimed: Some(3 * 1_048_576),
        }),
        Ok(PruneResult {
            deleted: vec![],
            reclaimed: None,
        }),
        Ok(PruneResult {
            deleted: vec![],
            reclaimed: Some(0),
        }),
    ]
    .into_iter()
    .enumerate()
    {
        let mode = if index == 0 || index == 2 {
            PruneMode::Anonymous
        } else {
            PruneMode::AllUnused
        };
        if index == 1 {
            h.click("volume-row-demo-database");
            h.key("ctrl-f");
            h.key("ctrl-a");
            h.type_text("demo-cache");
            h.key("escape");
            h.ctrl_click("volume-row-demo-cache");
            h.key("ctrl-f");
            h.key("ctrl-a");
            h.type_text("demo-database");
            h.key("escape");
        }
        h.click("volume-prune");
        h.click(if mode == PruneMode::Anonymous {
            "volume-prune-anonymous"
        } else {
            "volume-prune-unused"
        });
        let operation = h.observe(|v| v.volumes.pruning.unwrap());
        assert!(!h.observe(|v| v.volumes.prune_review));
        assert_eq!(h.drain_commands().iter().filter(|command| matches!(command, Command::PruneVolumes { mode: current_mode, operation: current } if *current == operation && *current_mode == mode)).count(), 1);
        h.click("volume-prune");
        h.click("volume-remove");
        assert!(!h.drain_commands().iter().any(|command| matches!(
            command,
            Command::PruneVolumes { .. }
                | Command::RemoveVolume { .. }
                | Command::RemoveVolumes { .. }
        )));
        h.deliver(Event::VolumesPruned {
            operation: operation + 1,
            result: Ok(PruneResult {
                deleted: vec![],
                reclaimed: Some(999),
            }),
        });
        assert_eq!(h.observe(|v| v.volumes.pruning), Some(operation));
        if index == 0 {
            h.capture("volumes-prune-pending");
            h.click("0-0");
            h.drain_commands();
            h.click("primary-action");
            assert!(
                !h.drain_commands()
                    .iter()
                    .any(|command| matches!(command, Command::Action { .. }))
            );
            h.click("0-1");
            let generation = h.observe(|v| v.images.generation);
            h.deliver(Event::Images {
                generation,
                result: Ok(vec![]),
            });
            h.drain_commands();
            h.click("image-prune");
            assert!(!h.observe(|v| v.images.prune_review));
            assert!(
                !h.drain_commands()
                    .iter()
                    .any(|command| matches!(command, Command::PruneImages { .. }))
            );
            h.click("0-2");
            deliver(h, &inventory);
        }
        if index == 2 {
            h.click("0-0");
        }
        h.deliver(Event::VolumesPruned { operation, result });
        assert!(h.observe(|v| v.volumes.pruning.is_none()));
        h.settle_ui(300);
        assert!(h.notification_count() > 0);
        if index == 0 {
            assert_eq!(h.observe(|v| v.volumes.items.len()), 4);
            h.capture("volumes-prune-failed-keeps-measurements");
        } else {
            assert_eq!(h.observe(|v| v.volumes.items.len()), 2);
            assert_eq!(
                h.observe(|v| v.volumes.selection.clone()),
                [original[1].name.clone()].into_iter().collect()
            );
            assert_eq!(
                h.observe(|v| v
                    .volumes
                    .items
                    .iter()
                    .find(|volume| volume.name == original[1].name)
                    .unwrap()
                    .size),
                Some(1_048_576)
            );
            inventory = original[1..].to_vec();
            if index == 1 {
                h.capture("volumes-prune-success");
            } else {
                h.click("0-2");
                h.capture(if index == 2 {
                    "volumes-prune-space-unavailable"
                } else {
                    "volumes-prune-zero-reclaimed"
                });
            }
        }
        let commands = h.drain_commands();
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::GatherVolumeSizes { .. })),
            "Pruning never regathers surviving volume sizes"
        );
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::Volumes { .. })),
            "Pruning reconciles the inventory"
        );
        h.dismiss_image_feedback();
        deliver(h, &inventory);
        assert_eq!(
            h.observe(|v| v
                .volumes
                .items
                .iter()
                .find(|volume| volume.name == original[1].name)
                .unwrap()
                .size),
            Some(1_048_576)
        );
    }
    h.click("volume-prune");
    h.click("volume-prune-unused");
    let operation = h.observe(|v| v.volumes.pruning.unwrap());
    h.deliver(Event::Disconnected("Fixture prune disconnect".into()));
    h.deliver(Event::VolumesPruned {
        operation,
        result: Ok(PruneResult {
            deleted: vec![original[1].name.clone()],
            reclaimed: Some(1_048_576),
        }),
    });
    assert!(h.observe(|v| v.volumes.pruning.is_none() && !v.volumes.prune_review));
    assert!(h.observe(|v| {
        v.volumes
            .items
            .iter()
            .any(|volume| volume.name == original[1].name)
    }));
    h.capture("volumes-prune-disconnected");
    h.dismiss_image_feedback();
    h.deliver(Event::Connected(connection));
    deliver(h, original);
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.click("volume-row-demo-cache");
    h.drain_commands();
}

fn verify_volume_sizes(h: &mut Harness, inventory: &[crate::volumes::Volume]) {
    use crate::volumes::VolumeSize;
    let measured = |sizes: [Option<u64>; 3]| {
        inventory
            .iter()
            .zip(sizes)
            .map(|(volume, size)| VolumeSize {
                name: volume.name.clone(),
                driver: volume.driver.clone(),
                mountpoint: volume.mountpoint.clone(),
                created: volume.created.clone(),
                size,
            })
            .collect::<Vec<_>>()
    };
    let values = |h: &Harness| {
        h.observe(|v| {
            v.volumes
                .items
                .iter()
                .map(|volume| volume.size)
                .collect::<Vec<_>>()
        })
    };
    let order = |h: &Harness| h.observe(|v| v.volumes.visible.clone());
    let no_gather = |commands: Vec<Command>| {
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::GatherVolumeSizes { .. })),
            "Refresh/navigation must never recalculate sizes"
        )
    };
    h.drain_commands();
    assert_eq!(values(h), vec![None; 3]);
    h.ctrl_click("volume-row-demo-database");
    let selection = h.observe(|v| v.volumes.selection.clone());
    h.click("volume-gather-sizes");
    let commands = h.drain_commands();
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::GatherVolumeSizes { request } => Some(*request),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, Command::GatherVolumeSizes { .. }))
            .count(),
        1
    );
    h.click("volume-gather-sizes");
    no_gather(h.drain_commands());
    h.capture("volumes-gathering-sizes");
    // Ordinary discovery continues while the independently queued calculation runs.
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.to_vec()),
    });
    h.deliver(Event::VolumeSizes {
        request: request + 1,
        result: Ok(measured([Some(999); 3])),
    });
    assert!(h.observe(|v| v.volumes.gathering == Some(request)));
    assert_eq!(values(h), vec![None; 3]);
    h.deliver(Event::VolumeSizes {
        request,
        result: Ok(measured([Some(10 * 1_048_576), Some(2 * 1_048_576), None])),
    });
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    assert!(h.observe(|v| v.volumes.gathering.is_none()));
    h.click("volume-sort-size");
    assert_eq!(order(h), vec![1, 0, 2]);
    h.capture("volumes-sizes-smallest-first");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let size = window.find("volume-size-demo-cache").bounds();
        let header = window.find("volume-sort-size").bounds();
        let row = window.find("volume-row-demo-cache").bounds();
        assert!((size.origin.x - header.origin.x).abs() < px(2.0));
        assert!(size.origin.y >= row.origin.y && size.bottom() <= row.bottom());
    })
    .unwrap();
    h.click("volume-sort-size");
    assert_eq!(order(h), vec![0, 1, 2]);
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selection);
    h.capture("volumes-sizes-largest-first");
    no_gather(h.drain_commands());
    let mut refresh = inventory.to_vec();
    for volume in &mut refresh {
        volume.size = Some(123);
    }
    h.key("ctrl-r");
    no_gather(h.drain_commands());
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(refresh.clone()),
    });
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    h.deliver(Event::Volumes {
        generation,
        result: Err("Fixture inventory failure".into()),
    });
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    h.click("0-0");
    h.click("0-2");
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.to_vec()),
    });
    no_gather(h.drain_commands());
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    // Failure preserves the last measurements; only another explicit click replaces them.
    h.click("volume-gather-sizes");
    let request = h.observe(|v| v.volumes.gathering.unwrap());
    h.drain_commands();
    h.deliver(Event::VolumeSizes {
        request,
        result: Err("Fixture Docker disk usage failure".into()),
    });
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    h.capture("volumes-size-error-keeps-measurements");
    h.click("volume-gather-sizes");
    let request = h.observe(|v| v.volumes.gathering.unwrap());
    h.drain_commands();
    h.click("0-0");
    h.deliver(Event::VolumeSizes {
        request,
        result: Ok(measured([Some(0), Some(20 * 1_048_576), None])),
    });
    h.click("0-2");
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.to_vec()),
    });
    assert_eq!(values(h), vec![Some(0), Some(20 * 1_048_576), None]);
    assert_eq!(order(h), vec![1, 0, 2]);
    no_gather(h.drain_commands());
    h.capture("volumes-sizes-regathered-after-workspace-change");
    refresh[1].created = Some("2026-10-14T00:00:00Z".into());
    h.deliver(Event::Volumes {
        generation,
        result: Ok(refresh),
    });
    assert_eq!(
        values(h),
        vec![Some(0), None, None],
        "Recreated volumes must not inherit a cached size"
    );
    h.click("volume-sort-name");
    h.click("volume-row-demo-cache");
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.to_vec()),
    });
}

fn verify_volume_sorting_and_selection(h: &mut Harness, original: &[crate::volumes::Volume]) {
    use crate::volumes::{RemovalResult, Volume};
    let now = chrono::Utc::now().timestamp();
    let mut inventory = original.to_vec();
    inventory[0].created = Some(
        chrono::DateTime::from_timestamp(now - 864000, 0)
            .unwrap()
            .to_rfc3339(),
    );
    inventory[1].created = Some(
        chrono::DateTime::from_timestamp(now - 172800, 0)
            .unwrap()
            .to_rfc3339(),
    );
    inventory[2].created = None;
    inventory.push(Volume {
        name: "unavailable".into(),
        created: Some("invalid".into()),
        ..original[0].clone()
    });
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.drain_commands();
    let assert_order = |h: &Harness, expected: &[usize]| {
        assert_eq!(
            h.observe(|v| v
                .volumes
                .visible
                .iter()
                .map(|index| v.volumes.items[*index].name.clone())
                .collect::<Vec<_>>()),
            expected
                .iter()
                .map(|index| inventory[*index].name.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            h.observe(|v| v.volumes.selected.clone()),
            Some(inventory[0].name.clone())
        );
    };
    for (header, ascending, descending) in [
        ("volume-sort-age", vec![1, 0, 2, 3], vec![0, 1, 2, 3]),
        ("volume-sort-driver", vec![0, 1, 3, 2], vec![2, 0, 1, 3]),
        ("volume-sort-references", vec![0, 2, 3, 1], vec![1, 0, 2, 3]),
        ("volume-sort-name", vec![0, 1, 2, 3], vec![3, 2, 1, 0]),
    ] {
        h.click(header);
        assert_order(h, &ascending);
        if header == "volume-sort-age" {
            h.capture("volumes-age-youngest-first");
        }
        h.click(header);
        assert_order(h, &descending);
        if header == "volume-sort-age" {
            h.capture("volumes-age-oldest-first");
        }
    }
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let row = window.find("volume-row-demo-cache").bounds();
        let age = window.find("volume-age-demo-cache").bounds();
        let header = window.find("volume-sort-age").bounds();
        assert!(age.origin.x > row.origin.x + row.size.width / 2.0);
        assert!(age.origin.y >= row.origin.y && age.bottom() <= row.bottom());
        assert!((age.origin.x - header.origin.x).abs() < px(2.0));
        let cache = window.find("volume-row-demo-cache").bounds();
        let database = window.find("volume-row-demo-database").bounds();
        assert!(
            database.origin.y < cache.origin.y,
            "Painted rows must follow descending name order"
        );
    })
    .unwrap();
    assert!(
        h.drain_commands().is_empty(),
        "Sorting must not send Docker operations"
    );
    // Ctrl-click supports multiple selection and clearing the final row.
    h.ctrl_click("volume-row-demo-database");
    h.ctrl_click("volume-row-remote-data");
    assert_eq!(h.observe(|v| v.volumes.selection.len()), 3);
    h.capture("volumes-multiple-selected");
    h.ctrl_click("volume-row-demo-cache");
    h.ctrl_click("volume-row-demo-database");
    h.ctrl_click("volume-row-remote-data");
    assert!(h.observe(|v| v.volumes.selection.is_empty()));
    h.click("volume-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.click("volume-row-demo-cache");
    h.ctrl_click("volume-row-demo-database");
    h.ctrl_click("volume-row-remote-data");
    let selected = h.observe(|v| v.volumes.selection.clone());
    h.click("volume-sort-driver");
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selected);
    h.key("ctrl-f");
    h.type_text("demo-cache");
    h.key("escape");
    assert_eq!(h.observe(|v| v.volumes.visible.len()), 1);
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selected);
    h.capture("volumes-multiple-hidden-selection");
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![
            inventory[2].clone(),
            inventory[3].clone(),
            inventory[1].clone(),
            inventory[0].clone(),
        ]),
    });
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selected);
    h.click("volume-remove");
    let names = selected.iter().cloned().collect::<Vec<_>>();
    assert_eq!(
        h.observe(|v| v.volumes.confirmation.clone()),
        Some(names.clone())
    );
    h.capture("volumes-multiple-hidden-review");
    for name in &names {
        assert!(
            h.cx.debug_bounds(h.window.into(), &format!("volume-removal-target-{name}"))
                .unwrap()
                .is_some()
        );
    }
    h.click("volume-cancel-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    assert!(!h.drain_commands().iter().any(|command| matches!(
        command,
        Command::RemoveVolume { .. } | Command::RemoveVolumes { .. }
    )));
    h.click("volume-remove");
    h.key("escape");
    h.settle_ui(300);
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h
        .drain_commands()
        .iter()
        .find_map(|command| match command {
            Command::RemoveVolumes {
                names: actual,
                operation,
            } if actual == &names => Some(*operation),
            _ => None,
        })
        .expect("Bulk removal must contain every reviewed name, including hidden selections");
    h.click("volume-remove");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveVolumes { .. }))
    );
    h.deliver(Event::VolumesRemoved {
        operation: operation + 1,
        results: vec![RemovalResult {
            name: inventory[0].name.clone(),
            result: Ok(()),
        }],
    });
    assert!(h.observe(|v| v.volumes.busy.is_some()));
    h.deliver(Event::VolumesRemoved {
        operation,
        results: vec![
            RemovalResult {
                name: inventory[0].name.clone(),
                result: Ok(()),
            },
            RemovalResult {
                name: inventory[1].name.clone(),
                result: Err("Volume is used by a stopped container".into()),
            },
            RemovalResult {
                name: inventory[2].name.clone(),
                result: Err("Fixture Docker conflict".into()),
            },
        ],
    });
    assert_eq!(
        h.observe(|v| v.volumes.selection.clone()),
        [inventory[1].name.clone(), inventory[2].name.clone()]
            .into_iter()
            .collect()
    );
    assert!(!h.observe(|v| {
        v.volumes
            .items
            .iter()
            .any(|volume| volume.name == inventory[0].name)
    }));
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory[1..].to_vec()),
    });
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.settle_ui(300);
    h.capture("volumes-multiple-partial-results");
    h.dismiss_image_feedback();
    h.click("volume-row-remote-data");
    h.ctrl_click("volume-row-unavailable");
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h.observe(|v| v.volumes.busy.as_ref().unwrap().1);
    h.deliver(Event::VolumesRemoved {
        operation,
        results: vec![
            RemovalResult {
                name: inventory[2].name.clone(),
                result: Ok(()),
            },
            RemovalResult {
                name: inventory[3].name.clone(),
                result: Ok(()),
            },
        ],
    });
    assert!(h.observe(|v| v.volumes.selection.is_empty() && v.volumes.selected.is_none()));
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![inventory[1].clone()]),
    });
    assert!(h.observe(|v| v.volumes.selection.is_empty()));
    h.capture("volumes-multiple-removed");
    h.dismiss_image_feedback();
    // Restore the ordinary single-selection flow through the same front door.
    h.deliver(Event::Volumes {
        generation,
        result: Ok(original.to_vec()),
    });
    h.click("volume-sort-name");
    h.click("volume-row-demo-cache");
}

#[test]
fn headless_gpu_volume_flow() {
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
    ]));
    verify_volumes_workspace(&mut h);
    let report = json!({
        "status": "passed", "renderer": "GPUI Kit gpui-fast WgpuHeadlessRenderer",
        "display": null, "wayland_display": null, "docker_worker_started": false,
        "logical_width": 1180, "logical_height": 820, "pixel_width": 2360, "pixel_height": 1640,
        "render_and_flow_seconds": started.elapsed().as_secs_f64(),
        "screenshots": h.screenshots, "frame_timings": h.frame_timings,
        "checks": ["Volumes sidebar and container session preservation", "name driver and project keyboard search", "Age column aligned with sortable headers", "Name Driver Age References ascending and descending with painted row order", "manual size gathering loading duplicate guard and numeric sorting with unavailable last", "size cache retained through Ctrl+R automatic inventory and workspace changes", "gather failure preserves measurements and explicit regather updates sorting", "recreated volume identity invalidates old measurement", "same-endpoint reconnect preserves sizes and disconnect rejects stale gathering", "different Docker endpoint clears cached sizes", "Age numeric ordering with unavailable last in both directions", "Ctrl-click multiple selection and clearing final row", "selection preserved through sorting filtering and fresh inventory", "hidden and protected selections included in review", "Cancel Escape and exact-name single/bulk confirmation", "pending duplicate and conflicting container controls blocked", "partial bulk results retain failed selections", "full bulk success clears selection", "stopped reference guard and container navigation", "10000-row virtualization and search", "Light theme", "stale generations and operation IDs rejected", "bulk disconnection invalidates pending state", "anonymous/all-unused prune dialog Cancel Escape and Enter guard", "anonymous and all-unused prune requests ignore selection and filter", "prune duplicate and conflicting container/image actions blocked", "prune success failure zero/unknown reclaim and size-cache reconciliation", "prune survives workspace change and ignores stale/disconnected completion"]
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
}
#[test]
fn headless_gpu_image_analysis_flow() {
    use super::image_details::ImageTab;
    use crate::image_analysis::{FileNode, Finding, Layer, Report};
    let started = Instant::now();
    let mut h = Harness::new();
    h.deliver(Event::Connected(Connection {
        name: "Headless fixture".into(),
        endpoint: "fixture://no-docker".into(),
    }));
    h.click("0-1");
    let generation = h.observe(|v| v.images.generation);
    let id = format!("sha256:{}", "a".repeat(64));
    let inventory = vec![crate::images::Image {
        id: id.clone(),
        tags: vec!["ferrydock-analysis-demo:latest".into()],
        size: Some(12 * 1_048_576),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 3600,
        digests: vec![],
        containers: vec![].into(),
    }];
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.drain_commands();
    h.click_tab("image-tabs", 3);
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Analysis));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::AnalyzeImage { .. })),
        "Analysis is explicitly requested"
    );
    h.capture("image-analysis-idle");
    h.click("image-analyze");
    let request = h.observe(|v| v.images.details.analysis.generation);
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::AnalyzeImage { id: current, generation } if current == &id && *generation == request)));
    h.capture("image-analysis-loading");
    h.click("image-analysis-cancel");
    assert!(
        h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::CloseImageAnalysis))
    );
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: request,
        result: Err("stale cancelled result".into()),
    });
    assert!(h.observe(|v| v.images.details.analysis.loading));
    let report = Arc::new(Report {
        unpacked_bytes: 12 * 1_048_576,
        wasted_bytes: 4 * 1_048_576,
        efficiency: Some(66),
        layers: vec![
            Layer {
                command: "COPY application /app".into(),
                unpacked_bytes: 8 * 1_048_576,
                files: vec![FileNode {
                    name: "app".into(),
                    bytes: 8 * 1_048_576,
                    change: "Added".into(),
                    children: vec![
                        FileNode {
                            name: "app-server".into(),
                            bytes: 4 * 1_048_576,
                            change: "Added".into(),
                            ..Default::default()
                        },
                        FileNode {
                            name: "build-cache".into(),
                            bytes: 4 * 1_048_576,
                            change: "Added".into(),
                            children: vec![FileNode {
                                name: "compiler.tmp".into(),
                                bytes: 4 * 1_048_576,
                                change: "Added".into(),
                                ..Default::default()
                            }],
                        },
                    ],
                }],
                ..Default::default()
            },
            Layer {
                command: "RUN rm -rf /app/build-cache".into(),
                unpacked_bytes: 1_048_576,
                files: vec![FileNode {
                    name: "app".into(),
                    change: "Modified".into(),
                    children: vec![FileNode {
                        name: "build-cache".into(),
                        bytes: 4 * 1_048_576,
                        change: "Removed".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            Layer {
                command: "CMD [\"/app/app-server\"]".into(),
                metadata_only: true,
                ..Default::default()
            },
        ],
        findings: vec![Finding {
            title: "Reclaim wasted space".into(),
            detail: "A later layer removes a build cache retained by an earlier layer.".into(),
            hint: "Remove temporary files in the same RUN instruction that creates them.".into(),
            heuristic: true,
        }],
        ..Default::default()
    });
    h.deliver(Event::ImageAnalysis {
        id: "wrong-image".into(),
        generation: active,
        result: Ok(report.clone()),
    });
    assert!(h.observe(|v| v.images.details.analysis.loading));
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Ok(report.clone()),
    });
    assert!(!h.observe(|v| v.images.details.analysis.loading));
    h.capture("image-analysis-files");
    h.click("analysis-file-2"); // Expand build-cache via Kit Tree input.
    h.capture("image-analysis-expanded-folder");
    assert!(
        h.cx.debug_bounds(h.window.into(), "analysis-file-3")
            .unwrap()
            .is_some(),
        "Expanded folder must expose its nested file"
    );
    h.click("analysis-layer-1");
    assert_eq!(h.observe(|v| v.images.details.analysis.layer), 1);
    h.capture("image-analysis-deleted-files");
    h.click_tab("analysis-content-tabs", 1);
    h.capture("image-analysis-findings");
    h.click_tab("analysis-content-tabs", 0);
    h.click("analysis-layer-2");
    h.capture("image-analysis-metadata-only");
    h.key("ctrl-1");
    h.key("ctrl-4");
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Analysis));
    assert!(h.observe(|v| v.images.details.analysis.result.is_some()));
    h.click("image-analyze");
    let empty_request = h.observe(|v| v.images.details.analysis.generation);
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: empty_request,
        result: Ok(Arc::new(Report {
            layers: vec![Layer {
                command: "LABEL empty=true".into(),
                metadata_only: true,
                ..Default::default()
            }],
            ..Default::default()
        })),
    });
    h.capture("image-analysis-empty-efficiency");
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Err("Fixture export failed: image disappeared".into()),
    });
    h.capture("image-analysis-failed");
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    let mut large = (*report).clone();
    large.layers = (0..2_000)
        .map(|index| Layer {
            command: format!("COPY fixture-{index} /app"),
            files: (0..10_000)
                .map(|i| FileNode {
                    name: format!("file-{i}"),
                    bytes: 1024,
                    change: "Added".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .take(1)
        .chain((1..2_000).map(|index| Layer {
            command: format!("LABEL fixture-layer={index}"),
            metadata_only: true,
            ..Default::default()
        }))
        .collect();
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Ok(Arc::new(large)),
    });
    h.capture("image-analysis-large");
    assert!(
        h.observe(|v| v.images.details.analysis.rows_rendered.get()) < 12,
        "Layer list must be virtualized"
    );
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    // Workspace navigation invalidates detail results; deliver a fresh inventory.
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory),
    });
    h.click_tab("image-tabs", 3);
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Ok(report.clone()),
    });
    h.capture("image-analysis-light");
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    h.key("ctrl-r");
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Ok(report.clone()),
    });
    assert!(
        h.observe(|v| v.images.details.analysis.result.is_none()),
        "Refresh rejects stale analysis"
    );
    h.deliver(Event::Disconnected("Fixture disconnect".into()));
    h.deliver(Event::ImageAnalysis {
        id,
        generation: active,
        result: Ok(report),
    });
    assert!(h.observe(|v| v.images.details.analysis.result.is_none()));
    h.capture("image-analysis-disconnected");
    let report = json!({ "status": "passed", "renderer": "GPUI Kit gpui-fast WgpuHeadlessRenderer", "docker_worker_started": false,
        "logical_width": 1180, "logical_height": 820, "pixel_width": 2360, "pixel_height": 1640,
        "screenshots": h.screenshots, "frame_timings": h.frame_timings, "render_and_flow_seconds": started.elapsed().as_secs_f64(),
        "checks": ["explicit Analyze request", "Cancel rejects old generation after retry", "wrong image rejected", "native Kit layer selection and tree expansion", "Files and Findings tabs", "metadata-only state", "zero-payload efficiency unavailable", "Ctrl+4 navigation and retained report", "failure retry", "2000 layers and 10000 files", "Light theme", "refresh and disconnect guards"] });
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
}

include!("networks_headless.rs");
include!("favorites_headless.rs");
include!("quick_run_headless.rs");
