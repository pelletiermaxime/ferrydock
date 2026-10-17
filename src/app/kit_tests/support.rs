use super::*;

pub(super) trait HeadlessObservation {
    fn debug_bounds(
        &mut self,
        window: gpui::AnyWindowHandle,
        selector: &str,
    ) -> anyhow::Result<Option<gpui::Bounds<gpui::Pixels>>>;
}

impl HeadlessObservation for TestAppContext {
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

struct TestContextShutdown(TestAppContext);

impl Drop for TestContextShutdown {
    fn drop(&mut self) {
        self.0.quit();
        self.0.run_until_parked();
    }
}

pub(super) struct Harness {
    // Release the external view before shutting down the owned app context.
    pub(super) view: Entity<FerryDock>,
    _shutdown: TestContextShutdown,
    pub(super) cx: TestAppContext,
    pub(super) window: WindowHandle<gpui_kit::base::Root>,
    pub(super) events: mpsc::Sender<Event>,
    pub(super) commands: mpsc::UnboundedReceiver<Command>,
    pub(super) evidence: PathBuf,
    pub(super) actions: Vec<Value>,
    pub(super) checkpoints: Vec<String>,
}

impl Harness {
    pub(super) fn new(parent: &TestAppContext) -> Self {
        Self::with_size(parent, 1180.0, 820.0)
    }

    pub(super) fn with_size(parent: &TestAppContext, width: f32, height: f32) -> Self {
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
        let cx = parent.new_app();
        let shutdown = TestContextShutdown(cx.clone());
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            crate::app::appearance::init_test(evidence.join("appearance.json"), cx);
        });
        let (backend, commands) = Backend::fixture();
        let (events, receiver) = mpsc::channel(256);
        let mut content = None;
        let window = cx
            .update(|cx| {
                cx.open_window(
                    gpui::WindowOptions {
                        window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                            origin: gpui::Point::default(),
                            size: size(px(width), px(height)),
                        })),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| {
                            FerryDock::with_backend(
                                backend,
                                receiver,
                                crate::app::favorites::Favorites::with_path(Ok(
                                    evidence.join("favorites.json")
                                )),
                                window,
                                cx,
                            )
                        });
                        view.update(cx, |view, cx| view.focus(window, cx));
                        content = Some(view.clone());
                        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
                    },
                )
            })
            .unwrap();
        let view = content.unwrap();
        cx.run_until_parked();
        Self {
            _shutdown: shutdown,
            cx,
            window,
            view,
            events,
            commands,
            evidence,
            actions: Vec::new(),
            checkpoints: Vec::new(),
        }
    }

    pub(super) fn observe<R>(&self, f: impl FnOnce(&FerryDock) -> R) -> R {
        self.cx.read_entity(&self.view, |view, _| f(view))
    }

    pub(super) fn notification_count(&mut self) -> usize {
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.notifications(cx).len()
            })
            .unwrap()
    }

    pub(super) fn dismiss_notification(&mut self) {
        // Kit checks visibility before moving the pointer to click. Reveal the
        // toast's hover-only close button through its visible parent first.
        self.actions
            .push(json!({"input": "hover", "selector": "notification"}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.hover("notification", cx);
            })
            .unwrap();
        self.cx.run_until_parked();
        self.click("close");
    }

    pub(super) fn settle_ui(&mut self, milliseconds: u64) {
        for _ in 0..milliseconds.div_ceil(50) {
            // Reduced motion removes wall-clock animation waits; lifecycle
            // timers and deferred focus still advance on the test clock.
            self.cx.executor().advance_clock(Duration::from_millis(50));
            self.cx.run_until_parked();
            self.cx
                .update_window(self.window.into(), |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
    }

    pub(super) fn dialog_open(&mut self) -> bool {
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.has_active_dialog(cx)
            })
            .unwrap()
    }

    pub(super) fn finish_input(&mut self, dialog_was_open: bool) {
        self.cx.run_until_parked();
        if dialog_was_open != self.dialog_open() {
            // Kit defers focus restoration on close; complete its timer
            // before the next input, even with reduced motion enabled.
            self.settle_ui(gpui_kit::component::dialog::ANIMATION_DURATION.as_millis() as u64);
        }
    }

    pub(super) fn click_tab(&mut self, bar: &'static str, index: usize) {
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

    pub(super) fn click_popup_item(&mut self, index: usize) {
        let dialog_was_open = self.dialog_open();
        self.actions
            .push(json!({"input": "menu-click", "index": index}));
        let executor = self.cx.foreground_executor().clone();
        executor.block_test(self.cx.wait_for(
            self.window.into(),
            Duration::from_secs(1),
            |window, _| {
                window
                    .try_find("popup-menu")
                    .is_some_and(|menu| menu.visible())
            },
        ));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.within("popup-menu").click(index, cx);
            })
            .unwrap();
        self.finish_input(dialog_was_open);
    }

    pub(super) fn inspection_value(&self) -> SharedString {
        self.cx.read_entity(&self.view, |view, cx| {
            view.containers.inspection.read(cx).value()
        })
    }

    pub(super) fn dock_height(&self) -> f32 {
        self.cx.read_entity(&self.view, |view, cx| {
            f32::from(view.dock.read(cx).sizes()[1])
        })
    }

    pub(super) fn drag_dock(&mut self, delta: f32) {
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

    pub(super) fn drag_list_scrollbar(&mut self, viewport: &'static str, to_end: bool) {
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

    pub(super) fn compose_values(&self) -> [SharedString; 4] {
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

    pub(super) fn assert_compose_focus(&mut self, index: usize) {
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

    pub(super) fn deliver(&mut self, event: Event) {
        self.events.try_send(event).unwrap();
        self.cx.executor().advance_clock(Duration::from_millis(40));
        self.cx.run_until_parked();
    }

    pub(super) fn key(&mut self, key: &str) {
        let dialog_was_open = self.dialog_open();
        self.actions.push(json!({"input": "key", "key": key}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.press(key, cx);
            })
            .unwrap();
        self.finish_input(dialog_was_open);
    }

    pub(super) fn type_text(&mut self, text: &str) {
        let dialog_was_open = self.dialog_open();
        self.actions.push(json!({"input": "text", "text": text}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.input(text, cx);
            })
            .unwrap();
        self.finish_input(dialog_was_open);
    }

    pub(super) fn click(&mut self, selector: &str) {
        self.click_button(selector, MouseButton::Left);
    }

    pub(super) fn dismiss_image_feedback(&mut self) {
        self.settle_ui(300);
        if self.notification_count() > 0 {
            self.dismiss_notification();
            self.settle_ui(300);
        }
    }

    pub(super) fn hover(&mut self, selector: &str, horizontal: f32) {
        self.actions
            .push(json!({"input": "hover", "selector": selector, "fraction": horizontal}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                let id = SharedString::from(selector.to_owned());
                if horizontal == 0.5 {
                    window.hover(id, cx);
                    return;
                }
                // Kit 0.7.3 only hovers at the center. Charts need distinct
                // sample positions, and long log rows need their visible text.
                window.render_frame(cx);
                let target = window.find(id);
                assert!(target.visible(), "Hover target {selector} must be visible");
                let bounds = target.bounds();
                let position = gpui::point(
                    bounds.left() + bounds.size.width * horizontal,
                    bounds.top() + bounds.size.height * 0.4,
                );
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

    pub(super) fn scroll_containers(&mut self, delta: f32) {
        self.actions
            .push(json!({"input": "wheel", "selector": "containers-pane", "delta": delta}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.scroll(
                    "containers-pane",
                    gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(delta))),
                    cx,
                );
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    pub(super) fn history_click_line(&mut self, line: usize, button: MouseButton) {
        self.click_button(&format!("history-row-{line}"), button);
    }

    pub(super) fn history_scroll(&mut self, x: f32, y: f32) {
        self.actions
            .push(json!({"input":"history wheel", "x":x,"y":y}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.scroll(
                    "image-history-table",
                    gpui::ScrollDelta::Pixels(gpui::point(px(x), px(y))),
                    cx,
                );
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    pub(super) fn tooltip_label(&mut self, selector: &'static str) -> String {
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.find(selector).label().unwrap().to_owned()
            })
            .unwrap()
    }

    pub(super) fn click_button(&mut self, selector: &str, button: MouseButton) {
        let dialog_was_open = self.dialog_open();
        self.actions
            .push(json!({"input": "click", "selector": selector, "button": format!("{button:?}")}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                let id = SharedString::from(selector.to_owned());
                match button {
                    MouseButton::Left => window.click(id, cx),
                    MouseButton::Right => window.right_click(id, cx),
                    _ => panic!("Unsupported test click button {button:?}"),
                }
            })
            .unwrap();
        self.finish_input(dialog_was_open);
    }

    pub(super) fn double_click(&mut self, selector: &str) {
        let dialog_was_open = self.dialog_open();
        self.actions
            .push(json!({"input": "double-click", "selector": selector}));
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.double_click(SharedString::from(selector.to_owned()), cx);
            })
            .unwrap();
        self.finish_input(dialog_was_open);
    }

    pub(super) fn ctrl_click(&mut self, selector: &str) {
        let dialog_was_open = self.dialog_open();
        // Kit 0.7.3's pointer helpers always use default modifiers. Keep this
        // adapter only for multi-selection; ordinary clicks use Kit above.
        let button = MouseButton::Left;
        let modifiers = Modifiers {
            control: true,
            ..Modifiers::none()
        };
        let bounds = self
            .cx
            .debug_bounds(self.window.into(), selector)
            .unwrap()
            .expect("Ctrl-click target");
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
        self.finish_input(dialog_was_open);
    }

    pub(super) fn checkpoint(&mut self, name: &str) {
        self.cx
            .update_window(self.window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        self.checkpoints.push(name.to_owned());
        self.write_actions();
    }

    fn write_actions(&self) {
        std::fs::write(
            self.evidence.join("actions.json"),
            serde_json::to_vec_pretty(&self.actions).unwrap(),
        )
        .unwrap();
    }

    pub(super) fn finish(&mut self, mut report: Value) {
        report["status"] = json!("passed");
        report["framework"] = json!("gpui-kit");
        report["real_gpu"] = json!(false);
        report["checkpoints"] = json!(self.checkpoints);
        report["action_count"] = json!(self.actions.len());
        self.write_actions();
        std::fs::write(
            self.evidence.join("headless.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }

    pub(super) fn drain_commands(&mut self) -> Vec<Command> {
        std::iter::from_fn(|| self.commands.try_recv().ok()).collect()
    }

    pub(super) fn require_visible(&mut self, selector: &str) {
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window
                        .find(SharedString::from(selector.to_owned()))
                        .visible(),
                    "Expected visible element {selector}"
                );
            })
            .unwrap();
    }
}

pub(super) fn container(id: &str, name: &str, state: &str, project: Option<&str>) -> Container {
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
