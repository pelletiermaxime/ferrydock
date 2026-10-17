//! Kit interaction tests with fixture workers, test clocks and no GPU.

use crate::app::{FerryDock, Tab};
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
    AppContext, Entity, Focusable, InputEvent, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, SharedString, TestAppContext, WindowHandle, px, size,
};

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;

mod containers;
mod favorites;
mod image_analysis;
mod image_details;
mod image_sorting;
mod images;
mod lifecycle;
mod networks;
mod project_cleanup;
mod quick_run;
mod support;
mod updates;
mod user_flow;
mod volume_pruning;
mod volume_selection;
mod volume_sizes;
mod volumes;

use containers::{verify_container_row_actions, verify_container_row_widths};
use image_details::verify_image_details;
use image_sorting::{verify_image_multiselection, verify_image_sorting};
use images::verify_images_workspace;
use project_cleanup::verify_project_cleanup;
use support::*;
use volume_pruning::verify_volume_pruning;
use volume_selection::verify_volume_sorting_and_selection;
use volume_sizes::verify_volume_sizes;
use volumes::verify_volumes_workspace;
