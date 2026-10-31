//! Dev tool: capture the screen through the Wayland `ext-image-copy-capture-v1`
//! protocol and write a PNG.
//!
//! The COSMIC compositor implements that protocol but not
//! `wlr-screencopy-unstable-v1`, and the XDG screenshot portal is not usable
//! in this environment, so `grim` and `cosmic-screenshot` cannot be used.
//!
//! Usage: `wl-shot <output.png>`

use std::fs::File;
use std::io::Write;

use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
    ext_image_copy_capture_manager_v1::RequestId,
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_manager_v1::ExtImageCaptureSourceManagerV1,
    ext_image_capture_source_manager_v1::Source,
};
use wayland_protocols::wp:: compositor::{Compositor, Surface};
use wayland_protocols::xdg::shell::client::xdg_wm_base::XdgWmBase;

mod dmabuf;
mod output;
mod shm;

struct App {
    manager: Option<ExtImageCopyCaptureManagerV1>,
    source: Option<ExtImageCaptureSourceManagerV1>,
    output: Option<wayland_protocols::ext::image_capture_source::v1::client::ext_output_manager_v1::ExtOutputManagerV1>,
    frame: Option<dmabuf::Frame>,
    buffer: Option<(wayland_client::protocol::wl_buffer::WlBuffer, u32, u32, u32, u32)>,
    shm_pool: Option<(shm::WlShm, wayland_client::protocol::wl_shm::WlShm)>,
    done: bool,
    failed: Option<String>,
}

impl App {
    fn new() -> Self {
        Self {
            manager: None,
            source: None,
            output: None,
            frame: None,
            buffer: None,
            shm_pool: None,
            done: false,
            failed: None,
        }
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &ExtImageCopyCaptureManagerV1,
        event: <ExtImageCopyCaptureManagerV1 as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let ExtImageCopyCaptureManagerV1::Event::Failed { reason } = event {
            state.failed = Some(reason);
            state.done = true;
        }
    }
}

impl Dispatch<ExtImageCaptureSourceManagerV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &ExtImageCaptureSourceManagerV1,
        event: <ExtImageCaptureSourceManagerV1 as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let ExtImageCaptureSourceManagerV1::Event::Failed { reason } = event {
            state.failed = Some(reason);
            state.done = true;
        }
    }
}

impl Dispatch<wayland_protocols::ext::image_capture_source::v1::client::ext_output_manager_v1::ExtOutputManagerV1, ()>
    for App
{
    fn event(
        state: &mut Self,
        _proxy: &wayland_protocols::ext::image_capture_source::v1::client::ext_output_manager_v1::ExtOutputManagerV1,
        event: <wayland_protocols::ext::image_capture_source::v1::client::ext_output_manager_v1::ExtOutputManagerV1 as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wayland_protocols::ext::image_capture_source::v1::client::ext_output_manager_v1::ExtOutputManagerV1::Event::Failed { reason } = event {
            state.failed = Some(reason);
            state.done = true;
        }
    }
}

impl Dispatch<Source, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &Source,
        event: <Source as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let Source::Event::Failed { reason } = event {
            state.failed = Some(reason);
            state.done = true;
        }
    }
}

impl Dispatch<RequestId, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &RequestId,
        event: <RequestId as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            RequestId::Event::Failed { reason } => {
                state.failed = Some(reason);
                state.done = true;
            }
            RequestId::Event::Source { source } => {
                // `source` is an object of the capture-source-manager interface.
                state.source = Some(unsafe {
                    ExtImageCaptureSourceManagerV1::from_raw(
                        source.as_ptr() as *mut (),
                        source.interface().as_ptr() as *mut (),
                    )
                });
            }
            RequestId::Event::Finished => {
                state.done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<dmabuf::DmabufFeedback, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &dmabuf::DmabufFeedback,
        event: <dmabuf::DmabufFeedback as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let dmabuf::DmabufFeedback::Event::Failed { reason } = event {
            state.failed = Some(reason);
            state.done = true;
        }
    }
}

impl Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for App {
    fn event(
        state: &mut Self,
        registry: &wayland_client::protocol::wl_registry::WlRegistry,
        event: <wayland_client::protocol::wl_registry::WlRegistry as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Proxy::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };

        match interface.as_str() {
            "ext_image_copy_capture_manager_v1" => {
                let manager: ExtImageCopyCaptureManagerV1 = registry
                    .bind(name, version.min(1), ())
                    .expect("bind copy capture manager");
                state.manager = Some(manager);
            }
            "ext_image_capture_source_manager_v1" => {
                let manager: ExtImageCaptureSourceManagerV1 = registry
                    .bind(name, version.min(1), ())
                    .expect("bind capture source manager");
                state.source = Some(manager);
            }
            "ext_output_manager_v1" => {
                let manager: wayland_protocols::ext::image_capture_source::v1::client::ext_output_manager_v1::ExtOutputManagerV1 =
                    registry.bind(name, version.min(1), ()).expect("bind output manager");
                state.output = Some(manager);
            }
            _ => {}
        }
    }
}

impl Dispatch<wayland_client::protocol::wl_compositor::WlCompositor, ()> for App {}
impl Dispatch<Surface, ()> for App {}
impl Dispatch<XdgWmBase, ()> for App {}
impl Dispatch<wayland_client::protocol::wl_shm::WlShm, ()> for App {}
impl Dispatch<wayland_client::protocol::wl_shm_pool::WlShmPool, ()> for App {}
impl Dispatch<wayland_client::protocol::wl_buffer::WlBuffer, ()> for App {}
impl Dispatch<wayland_client::protocol::wl_callback::WlCallback, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &wayland_client::protocol::wl_callback::WlCallback,
        event: <wayland_client::protocol::wl_callback::WlCallback as Proxy>::Event,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_callback::WlCallback::Event::Done { .. } = event {
            state.done = true;
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "shot.png".to_string());

    let conn = Connection::connect_to_env().expect("connect to wayland");
    let (mut registry, _queue) = conn.display().get_registry(&conn, ());
    let mut event_queue: EventQueue<App> = conn.new_event_queue();

    let qh = event_queue.handle();
    let globals = conn.display().get_registry(&conn, ());
    let _ = &mut registry;
    let _ = globals;

    // Round-trip once so that the registry globals are delivered.
    event_queue
        .roundtrip(&mut App::new())
        .expect("initial roundtrip");

    let mut app = App::new();
    registry.dispatch_pending(&mut app);

    let Some(manager) = app.manager.clone() else {
        eprintln!("ext_image_copy_capture_manager_v1 not supported by this compositor");
        std::process::exit(2);
    };
    let Some(source_manager) = app.source.clone() else {
        eprintln!("ext_image_capture_source_manager_v1 not supported");
        std::process::exit(2);
    };
    let Some(output_manager) = app.output.clone() else {
        eprintln!("ext_output_manager_v1 not supported");
        std::process::exit(2);
    };

    let output = output_manager
        .get_output(&output_manager.get_outputs()[0], &qh, ())
        .expect("output source");

    let request = manager.create_request(&output, &qh, ());
    event_queue.roundtrip(&mut app).ok();
    registry.dispatch_pending(&mut app);

    // The request event gives us the source object.
    let Some(source_manager) = app.source.clone() else {
        eprintln!("no source manager after request");
        std::process::exit(3);
    };
    let _ = source_manager;

    // Find the source via the request's events (dispatched in the roundtrip).
    let Some(capture_source) = app.capture_source.take() else {
        eprintln!("no capture source");
        std::process::exit(3);
    };

    let params = dmabuf::create_params(&qh);
    let session = manager.create_session(&capture_source, &params, &qh, ());
    let _ = session;

    event_queue.roundtrip(&mut app).ok();
    registry.dispatch_pending(&mut app);

    if let Some(reason) = &app.failed {
        eprintln!("capture failed: {reason}");
        std::process::exit(4);
    }

    println!("captured (no encoder yet): {path}");
    let _ = File::create(path);
}
