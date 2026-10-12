//! Real GPU rendering and GPUI input dispatch with a fixture Docker boundary.
//! No compositor, Docker worker, OCR, or application state setters drive the UI.

use super::{FerryDock, TEXT, Tab};
use crate::{
    docker::{Action, Backend, Command, Connection, Event},
    model::{Container, groups},
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    AppContext, Entity, HeadlessAppContext, InputEvent, Keystroke, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, WindowHandle, px, rgb, size,
};

use image::{
    ImageEncoder, RgbaImage,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

struct Harness {
    // Rust drops fields in declaration order. Release this external handle
    // before the context shuts down and checks for leaked entities.
    view: Entity<FerryDock>,
    cx: HeadlessAppContext,
    window: WindowHandle<FerryDock>,
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
        let mut cx =
            HeadlessAppContext::with_platform(platform.text_system(), Arc::new(()), || {
                // CE's default factory still returns None on Linux. Never fall back
                // to a no-op renderer: GPU initialization failure must fail the test.
                Some(Box::new(
                    gpui_wgpu::WgpuHeadlessRenderer::new().expect("headless GPU renderer"),
                ))
            });
        let (backend, commands) = Backend::fixture();
        let (events, receiver) = mpsc::channel(256);
        let window = cx
            .open_window(size(px(1180.0), px(820.0)), |window, cx| {
                let view = cx.new(|cx| FerryDock::with_backend(backend, receiver, cx));
                view.update(cx, |view, cx| view.focus(window, cx));
                view
            })
            .unwrap();
        let view = cx.read_window(&window, |view, _| view).unwrap();
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

    fn deliver(&mut self, event: Event) {
        self.events.try_send(event).unwrap();
        self.cx.advance_clock(Duration::from_millis(40));
        self.cx.run_until_parked();
    }

    fn key(&mut self, key: &str) {
        self.actions.push(json!({"input": "key", "key": key}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn type_text(&mut self, text: &str) {
        self.actions.push(json!({"input": "text", "text": text}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                for ch in text.chars() {
                    window.dispatch_keystroke(Keystroke::parse(&ch.to_string()).unwrap(), cx);
                }
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn click(&mut self, selector: &str) {
        // Bounds come from the painted element, never desktop coordinates.
        let bounds = self
            .cx
            .debug_bounds(self.window.into(), selector)
            .unwrap()
            .unwrap_or_else(|| panic!("No rendered control {selector}"));
        let position = bounds.center();
        self.actions
            .push(json!({"input": "click", "selector": selector,
            "x": f32::from(position.x), "y": f32::from(position.y)}));
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
                window.dispatch_event(
                    MouseDownEvent {
                        position,
                        button: MouseButton::Left,
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
                        button: MouseButton::Left,
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
        PngEncoder::new_with_quality(file, CompressionType::Fast, FilterType::NoFilter)
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
        h.observe(|v| v.selected.clone()).as_deref(),
        Some("fixture-probe-running")
    );
    let generation = h.observe(|v| v.generation);
    h.deliver(Event::Log {
        generation,
        data: b"HEADLESS GPU FIXTURE READY\nA real frame, without a desktop window.\n".to_vec(),
    });
    let initial = h.capture("containers-and-logs");
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
        !h.cx
            .glyph_bounds(h.window.into(), gpui::rgb_to_hsla(rgb(TEXT)))
            .unwrap()
            .is_empty(),
        "Real font glyphs must be painted"
    );

    h.key("ctrl-f");
    h.type_text("no-matching-container");
    assert_eq!(
        h.observe(|v| groups(&v.containers, v.running_only, &v.filter).len()),
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
    h.type_text("headless-fixture");
    h.key("escape");
    h.key("down");
    assert_eq!(h.observe(|v| v.filter.clone()), "headless-fixture");
    assert_eq!(
        h.observe(|v| groups(&v.containers, v.running_only, &v.filter)
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
    assert!(!h.observe(|v| v.log_follow));
    h.capture("logs-paused");
    h.click("follow");
    assert!(h.observe(|v| v.log_follow));

    h.drain_commands();
    h.click("primary-action");
    assert!(h.drain_commands().iter().any(|command| matches!(command,
        Command::Action { id, action: Action::Stop } if id == "fixture-probe-running")));
    h.capture("stop-requested");
    h.deliver(Event::ActionFinished {
        id: "fixture-probe-running".into(),
        result: Ok(()),
    });

    // Escape left the main view focused; Enter opens an embedded session.
    h.key("enter");
    assert!(h.observe(|v| v.tab == Tab::Terminal));
    let terminal_generation = h.observe(|v| v.terminal_generation);
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

    let report = json!({
        "status": "passed", "renderer": "GPUI-CE WgpuHeadlessRenderer",
        "display": null, "wayland_display": null, "docker_worker_started": false,
        "logical_width": 1180, "logical_height": 820,
        "pixel_width": terminal_image.width(), "pixel_height": terminal_image.height(),
        "render_and_flow_seconds": started.elapsed().as_secs_f64(),
        "changed_pixels_after_search_lower_bound": changed_pixels, "screenshots": h.screenshots,
        "frame_timings": h.frame_timings,
        "checks": ["fixture discovery", "keyboard search", "empty matches", "restored selection",
            "log follow pointer toggle", "stop command at Docker boundary", "terminal ANSI rendering", "terminal input routing"]
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
