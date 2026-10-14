//! Application composition. Feature modules own their state, controls, and views.
use crate::docker::{Backend, Connection, Event};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    input::{EditorState, InputEvent as SearchInputEvent, InputState},
    resizable::ResizableState,
};
use std::time::{Duration, Instant};

mod appearance;
mod container_list;
mod container_logs;
mod containers;
mod events;
mod images;
mod metrics;
mod navigation;
mod project_logs;
mod projects;
mod resource_menu;
mod shell;
mod terminal;
pub(crate) mod ui;

use container_logs::ContainerLogs;
use containers::Containers;
use project_logs::ProjectLogs;
use projects::Projects;
use terminal::Terminal;
pub use ui::init_ui;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Logs,
    Terminal,
    Inspect,
    Compose,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Self::Logs => "Logs",
            Self::Terminal => "Terminal",
            Self::Inspect => "Inspect",
            Self::Compose => "Project",
        }
    }
}

pub struct FerryDock {
    backend: Backend,
    events: tokio::sync::mpsc::Receiver<Event>,
    connection: Option<Connection>,
    connection_error: Option<String>,
    containers: Containers,
    images: images::Images,
    images_open: bool,
    metrics: metrics::Metrics,
    container_logs: ContainerLogs,
    terminal: Terminal,
    projects: Projects,
    project_logs: ProjectLogs,
    tab: Tab,
    focus: FocusHandle,
    dock: Entity<ResizableState>,
    settings_open: bool,
}

impl FerryDock {
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (backend, events) = Backend::start();
        Self::with_backend(backend, events, window, cx)
    }

    fn with_backend(
        backend: Backend,
        events: tokio::sync::mpsc::Receiver<Event>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search containers…  Ctrl+F"));
        let search_focus = search_input.focus_handle(cx);
        cx.subscribe(&search_input, |this, input, event, cx| {
            if matches!(event, SearchInputEvent::Change) {
                this.containers.filter = input.read(cx).value().to_string();
                cx.notify();
            }
        })
        .detach();
        let inspection = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .folding(true)
                .soft_wrap(false)
        });
        let dock = cx.new(|_| ResizableState::default());
        cx.observe(&dock, |_, _, cx| cx.notify()).detach();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let mut changed = this.metrics.expire();
                        let started = Instant::now();
                        for _ in 0..64 {
                            match this.events.try_recv() {
                                Ok(event) => {
                                    this.handle_event(event, window, cx);
                                    changed = true;
                                }
                                Err(_) => break,
                            }
                            if started.elapsed() >= Duration::from_millis(8) {
                                break;
                            }
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let focus = cx.focus_handle();
        let compose_focus = cx.focus_handle();
        let images = images::Images::new(window, cx);
        Self {
            backend,
            events,
            connection: None,
            connection_error: None,
            containers: Containers::new(search_input, search_focus, inspection),
            images,
            images_open: false,
            metrics: metrics::Metrics::default(),
            container_logs: ContainerLogs::default(),
            terminal: Terminal::default(),
            projects: Projects::new(compose_focus),
            project_logs: ProjectLogs::new(window, cx),
            tab: Tab::Logs,
            focus,
            dock,
            settings_open: false,
        }
    }
}

#[cfg(all(test, feature = "headless-tests", target_os = "linux"))]
#[path = "headless_tests.rs"]
mod headless_tests;
