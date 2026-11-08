use std::{env, fs::File, io::BufWriter, path::Path, sync::Mutex};

use anyhow::{Context, Result, bail, ensure};
use cosmic_client_toolkit::{
    cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State,
    sctk::{
        self,
        registry::{ProvidesRegistryState, RegistryState},
        shm::{Shm, ShmHandler, raw::RawPool},
    },
    screencopy::{
        CaptureFrame, CaptureOptions, CaptureSession, CaptureSource, FailureReason, Formats, Frame,
        Rect, ScreencopyFrameData, ScreencopyFrameDataExt, ScreencopyHandler, ScreencopySessionData,
        ScreencopyState,
    },
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
    toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
    wayland_client::{
        Connection, QueueHandle, WEnum, delegate_noop,
        globals::registry_queue_init,
        protocol::{wl_buffer, wl_seat, wl_shm},
    },
    wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
};
use cosmic_client_toolkit::cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1 as Capability;

const APP_ID: &str = "dev.ferrydock.FerryDock";
const TITLE: &str = "FerryDock";
const CAPTURE_GLOBALS: [&str; 2] = [
    "ext_foreign_toplevel_image_capture_source_manager_v1",
    "ext_image_copy_capture_manager_v1",
];

struct Desktop {
    registry: RegistryState,
    windows: ToplevelInfoState,
    manager: ToplevelManagerState,
    can_activate: bool,
    snapshot_done: bool,
    shm: Shm,
    capture: ScreencopyState,
    pixels: Option<Result<Pixels>>,
}

struct Pixels {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

pub fn run() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let command = args.first().and_then(|arg| arg.to_str());
    ensure!(
        matches!(
            (command, args.len()),
            (Some("list"), 1) | (Some("activate" | "focused"), 2) | (Some("capture"), 3)
        ),
        "usage: ferrydock-cosmic-window list | activate ID | focused ID | capture ID PNG"
    );

    let connection = Connection::connect_to_env().context("connect to Wayland")?;
    let (globals, mut queue) = registry_queue_init(&connection)?;
    let qh = queue.handle();
    for interface in CAPTURE_GLOBALS {
        ensure!(
            globals
                .contents()
                .with_list(|list| list.iter().any(|global| global.interface == interface)),
            "compositor does not expose {interface}"
        );
    }
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ())?;
    let registry = RegistryState::new(&globals);
    let mut windows =
        ToplevelInfoState::try_new(&registry, &qh).context("list Wayland toplevels")?;
    ensure!(
        windows.cosmic_toplevel_info.is_some(),
        "COSMIC toplevel information is unavailable"
    );
    if command == Some("list") {
        // The standard list publishes identity immediately, even on an empty
        // desktop. COSMIC publishes focus later during compositor refresh.
        windows.cosmic_toplevel_info.take();
    }
    let mut desktop = Desktop {
        windows,
        manager: ToplevelManagerState::try_new(&registry, &qh)
            .context("bind COSMIC window manager")?,
        registry,
        can_activate: false,
        snapshot_done: false,
        shm: Shm::bind(&globals, &qh)?,
        capture: ScreencopyState::new(&globals, &qh),
        pixels: None,
    };
    queue.roundtrip(&mut desktop)?;
    if command != Some("list") {
        // The calling harness bounds this wait if the compositor stops responding.
        while !desktop.snapshot_done {
            queue.blocking_dispatch(&mut desktop)?;
        }
    }
    ensure!(
        desktop.can_activate,
        "COSMIC does not allow toplevel activation"
    );

    let mut matching = desktop
        .windows
        .toplevels()
        .filter(|info| info.app_id == APP_ID && info.title == TITLE && !info.identifier.is_empty());
    if command == Some("list") {
        for info in matching {
            println!("{}", info.identifier);
        }
        return Ok(());
    }
    let id = args[1].to_str().context("invalid toplevel ID")?;
    let window = matching
        .find(|window| window.identifier == id)
        .cloned()
        .context("owned FerryDock toplevel disappeared")?;
    drop(matching);
    if command == Some("activate") {
        let handle = window
            .cosmic_toplevel
            .as_ref()
            .context("missing COSMIC toplevel handle")?;
        desktop.manager.manager.activate(handle, &seat);
        queue.roundtrip(&mut desktop)?;
        return Ok(());
    }

    ensure!(
        window.state.contains(&State::Activated),
        "FerryDock toplevel is not focused"
    );
    if command == Some("capture") {
        let _session = desktop.capture.capturer().create_session(
            &CaptureSource::Toplevel(window.foreign_toplevel.clone()),
            CaptureOptions::empty(),
            &qh,
            ScreencopySessionData::default(),
        )?;
        while desktop.pixels.is_none() {
            queue.blocking_dispatch(&mut desktop)?;
        }
        let pixels = desktop.pixels.context("capture did not return pixels")??;
        let file = BufWriter::new(File::create(Path::new(&args[2]))?);
        let mut encoder = png::Encoder::new(file, pixels.width, pixels.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&pixels.rgb)?;
        writer.finish()?;
    }
    Ok(())
}

impl ProvidesRegistryState for Desktop {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    sctk::registry_handlers!();
}

impl ShmHandler for Desktop {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ToplevelInfoHandler for Desktop {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.windows
    }
    fn new_toplevel(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ExtForeignToplevelHandleV1,
    ) {
    }
    fn update_toplevel(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ExtForeignToplevelHandleV1,
    ) {
    }
    fn toplevel_closed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ExtForeignToplevelHandleV1,
    ) {
    }
    fn info_done(&mut self, _: &Connection, _: &QueueHandle<Self>) {
        self.snapshot_done = true;
    }
}

impl ToplevelManagerHandler for Desktop {
    fn toplevel_manager_state(&mut self) -> &mut ToplevelManagerState {
        &mut self.manager
    }
    fn capabilities(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        capabilities: Vec<WEnum<Capability>>,
    ) {
        self.can_activate = capabilities.contains(&WEnum::Value(Capability::Activate));
    }
}

struct FrameBuffer {
    data: ScreencopyFrameData,
    pool: Mutex<RawPool>,
    width: u32,
    height: u32,
    format: wl_shm::Format,
}

impl ScreencopyFrameDataExt for FrameBuffer {
    fn screencopy_frame_data(&self) -> &ScreencopyFrameData {
        &self.data
    }
}

impl Desktop {
    fn capture_frame(
        &self,
        qh: &QueueHandle<Self>,
        session: &CaptureSession,
        formats: &Formats,
    ) -> Result<()> {
        use wl_shm::Format::{Abgr8888, Argb8888, Xbgr8888, Xrgb8888};
        let format = [Xrgb8888, Argb8888, Xbgr8888, Abgr8888]
            .into_iter()
            .find(|format| formats.shm_formats.contains(format))
            .context("no supported shared-memory pixel format")?;
        let (width, height) = formats.buffer_size;
        let pixels = u64::from(width) * u64::from(height);
        ensure!(
            pixels > 0 && pixels <= 16 * 1024 * 1024,
            "capture exceeds 64 MiB"
        );
        let mut pool = RawPool::new(pixels as usize * 4, &self.shm)?;
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            width as i32 * 4,
            format,
            (),
            qh,
        );
        let damage = Rect {
            x: 0,
            y: 0,
            width: width as i32,
            height: height as i32,
        };
        session.capture(
            &buffer,
            &[damage],
            qh,
            FrameBuffer {
                data: ScreencopyFrameData::default(),
                pool: Mutex::new(pool),
                width,
                height,
                format,
            },
        );
        Ok(())
    }
}

impl ScreencopyHandler for Desktop {
    fn screencopy_state(&mut self) -> &mut ScreencopyState {
        &mut self.capture
    }

    fn init_done(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        session: &CaptureSession,
        formats: &Formats,
    ) {
        if let Err(error) = self.capture_frame(qh, session, formats) {
            self.pixels = Some(Err(error));
        }
    }

    fn stopped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &CaptureSession) {
        self.pixels = Some(Err(anyhow::anyhow!("COSMIC ended the capture session")));
    }

    fn ready(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        frame: &CaptureFrame,
        metadata: Frame,
    ) {
        self.pixels = Some(read_frame(frame, metadata));
    }

    fn failed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &CaptureFrame,
        reason: WEnum<FailureReason>,
    ) {
        self.pixels = Some(Err(anyhow::anyhow!("COSMIC capture failed: {reason:?}")));
    }
}

fn read_frame(frame: &CaptureFrame, metadata: Frame) -> Result<Pixels> {
    use cosmic_client_toolkit::wayland_client::protocol::wl_output::Transform;
    ensure!(
        metadata.transform == WEnum::Value(Transform::Normal),
        "unexpected toplevel image transform"
    );
    let buffer = frame
        .data::<FrameBuffer>()
        .context("missing frame buffer")?;
    let mut pool = buffer.pool.lock().expect("frame buffer lock poisoned");
    let mut rgb = Vec::with_capacity(buffer.width as usize * buffer.height as usize * 3);
    for &pixel in pool.mmap().as_chunks::<4>().0 {
        let [_, red, green, blue] = u32::from_ne_bytes(pixel).to_be_bytes();
        match buffer.format {
            wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => rgb.extend([red, green, blue]),
            wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => rgb.extend([blue, green, red]),
            _ => bail!("unexpected capture format"),
        }
    }
    Ok(Pixels {
        width: buffer.width,
        height: buffer.height,
        rgb,
    })
}

sctk::delegate_registry!(Desktop);
sctk::delegate_shm!(Desktop);
cosmic_client_toolkit::delegate_toplevel_info!(Desktop);
cosmic_client_toolkit::delegate_toplevel_manager!(Desktop);
cosmic_client_toolkit::delegate_screencopy!(Desktop);
delegate_noop!(Desktop: ignore wl_buffer::WlBuffer);
delegate_noop!(Desktop: ignore wl_seat::WlSeat);
