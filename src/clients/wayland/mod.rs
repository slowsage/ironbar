mod macros;
mod wl_output;
mod wl_seat;

use crate::error::{ERR_CHANNEL_RECV, ExitCode};
use crate::{arc_mut, lock, register_client, spawn, spawn_blocking};
use std::process::exit;
use std::sync::{Arc, Mutex};

use crate::channels::SyncSenderExt;
use calloop_channel::Event::Msg;
use cfg_if::cfg_if;
use color_eyre::{Help, Report};
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel as calloop_channel;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::{
    delegate_output, delegate_registry, delegate_seat, registry_handlers,
};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, trace};
use wayland_client::globals::registry_queue_init;
use wayland_client::{Connection, Proxy, QueueHandle};
pub use wl_output::{OutputEvent, OutputEventType};
cfg_if! {
  if #[cfg(feature = "inhibit")] {
    use chrono::{DateTime, Utc};
    use wayland_client::protocol::wl_surface::WlSurface;
    use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
    use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1;

    #[derive(Debug)]
    struct InhibitState {
      manager: ZwpIdleInhibitManagerV1,
        inhibitor: Option<ZwpIdleInhibitorV1>,
        expiry: Option<DateTime<Utc>>,
      }
  }
}

cfg_if! {
    if #[cfg(any(feature = "focused", feature = "launcher"))] {
        mod wlr_foreign_toplevel;
        use crate::{delegate_foreign_toplevel_handle, delegate_foreign_toplevel_manager};
        use wlr_foreign_toplevel::manager::ToplevelManagerState;
        pub use wlr_foreign_toplevel::{ToplevelEvent, ToplevelHandle, ToplevelInfo};

    }
}

cfg_if! {
    if #[cfg(feature = "clipboard")] {
        mod wlr_data_control;

        use crate::{delegate_data_control_device, delegate_data_control_device_manager, delegate_data_control_offer, delegate_data_control_source};
        use self::wlr_data_control::device::DataControlDevice;
        use self::wlr_data_control::manager::DataControlDeviceManagerState;
        use self::wlr_data_control::source::CopyPasteSource;
        use wayland_client::protocol::wl_seat::WlSeat;

        pub use wlr_data_control::{ClipboardItem, ClipboardValue};

        #[derive(Debug)]
        pub struct DataControlDeviceEntry {
            seat: WlSeat,
            device: DataControlDevice,
        }
    }
}

fn get_gtk_wayland_connection() -> Connection {
    use gdk4_wayland::prelude::*;

    let display = gtk::gdk::Display::default()
        .expect("GTK Display not initialized");

    let wl_display_gdk = display
        .downcast::<gdk4_wayland::WaylandDisplay>()
        .expect("Not a Wayland display");

    let wl_display = wl_display_gdk
        .wl_display()
        .expect("Failed to get wl_display");

    let backend = wl_display.backend()
        .upgrade()
        .expect("Backend destroyed");

    Connection::from_backend(backend)
}

#[derive(Debug)]
pub enum Event {
    Output(OutputEvent),
    #[cfg(any(feature = "focused", feature = "launcher"))]
    Toplevel(ToplevelEvent),
    #[cfg(feature = "clipboard")]
    Clipboard(ClipboardItem),
}

#[derive(Debug)]
pub enum Request {
    Roundtrip,

    #[cfg(feature = "ipc")]
    OutputInfoAll,

    #[cfg(any(feature = "focused", feature = "launcher"))]
    ToplevelInfoAll,
    #[cfg(feature = "launcher")]
    ToplevelFocus(usize),
    #[cfg(feature = "launcher")]
    ToplevelMinimize(usize),

    #[cfg(feature = "clipboard")]
    CopyToClipboard(ClipboardItem),
    #[cfg(feature = "clipboard")]
    ClipboardItem,

    #[cfg(feature = "inhibit")]
    StartInhibit(std::time::Duration, wayland_client::backend::ObjectId),
    #[cfg(feature = "inhibit")]
    StopInhibit,
    #[cfg(feature = "inhibit")]
    GetInhibitExpiry,
}

#[derive(Debug)]
pub enum Response {
    /// An empty success response
    Ok,

    #[cfg(feature = "ipc")]
    OutputInfoAll(Vec<smithay_client_toolkit::output::OutputInfo>),

    #[cfg(any(feature = "focused", feature = "launcher"))]
    ToplevelInfoAll(Vec<ToplevelInfo>),

    #[cfg(feature = "clipboard")]
    ClipboardItem(Option<ClipboardItem>),

    #[cfg(feature = "inhibit")]
    InhibitExpiry(Option<DateTime<Utc>>),
}

#[derive(Debug)]
#[allow(dead_code)]
struct BroadcastChannel<T>(broadcast::Sender<T>, Arc<Mutex<broadcast::Receiver<T>>>);

impl<T> From<(broadcast::Sender<T>, broadcast::Receiver<T>)> for BroadcastChannel<T> {
    fn from(value: (broadcast::Sender<T>, broadcast::Receiver<T>)) -> Self {
        Self(value.0, arc_mut!(value.1))
    }
}

#[derive(Debug)]
pub struct Client {
    tx: calloop_channel::Sender<Request>,
    rx: Arc<Mutex<std::sync::mpsc::Receiver<Response>>>,

    output_channel: BroadcastChannel<OutputEvent>,
    #[cfg(any(feature = "focused", feature = "launcher"))]
    toplevel_channel: BroadcastChannel<ToplevelEvent>,
    #[cfg(feature = "clipboard")]
    clipboard_channel: BroadcastChannel<ClipboardItem>,
}

impl Client {
    pub(crate) fn new() -> Self {
        let (event_tx, mut event_rx) = mpsc::channel(32);

        let (request_tx, request_rx) = calloop_channel::channel();
        let (response_tx, response_rx) = std::sync::mpsc::channel();

        let output_channel = broadcast::channel(32);
        #[cfg(any(feature = "focused", feature = "launcher"))]
        let toplevel_channel = broadcast::channel(32);

        #[cfg(feature = "clipboard")]
        let clipboard_channel = broadcast::channel(32);

        // Get GTK's Wayland connection on main thread before spawn_blocking
        let conn = get_gtk_wayland_connection();

        spawn_blocking(move || {
            Environment::spawn(conn, event_tx, request_rx, response_tx);
        });

        // listen to events
        {
            let output_tx = output_channel.0.clone();
            #[cfg(any(feature = "focused", feature = "launcher"))]
            let toplevel_tx = toplevel_channel.0.clone();

            #[cfg(feature = "clipboard")]
            let clipboard_tx = clipboard_channel.0.clone();

            spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    match event {
                        Event::Output(event) => output_tx.send_expect(event),
                        #[cfg(any(feature = "focused", feature = "launcher"))]
                        Event::Toplevel(event) => toplevel_tx.send_expect(event),
                        #[cfg(feature = "clipboard")]
                        Event::Clipboard(item) => clipboard_tx.send_expect(item),
                    }
                }
            });
        }

        Self {
            tx: request_tx,
            rx: arc_mut!(response_rx),

            output_channel: output_channel.into(),
            #[cfg(any(feature = "focused", feature = "launcher"))]
            toplevel_channel: toplevel_channel.into(),
            #[cfg(feature = "clipboard")]
            clipboard_channel: clipboard_channel.into(),
        }
    }

    /// Sends a request to the environment event loop,
    /// and returns the response.
    fn send_request(&self, request: Request) -> Response {
        self.tx.send_expect(request);
        lock!(self.rx).recv().expect(ERR_CHANNEL_RECV)
    }

    /// Sends a round-trip request to the client,
    /// forcing it to send/receive any events in the queue.
    pub(crate) fn roundtrip(&self) -> Response {
        self.send_request(Request::Roundtrip)
    }

    #[cfg(feature = "inhibit")]
    pub fn start_inhibit(
        &self,
        duration: std::time::Duration,
        surface_id: wayland_client::backend::ObjectId,
    ) {
        self.send_request(Request::StartInhibit(duration, surface_id));
    }

    #[cfg(feature = "inhibit")]
    pub fn stop_inhibit(&self) {
        self.send_request(Request::StopInhibit);
    }

    #[cfg(feature = "inhibit")]
    pub fn inhibit_expiry(&self) -> Option<DateTime<Utc>> {
        match self.send_request(Request::GetInhibitExpiry) {
            Response::InhibitExpiry(expiry) => expiry,
            _ => None,
        }
    }
}

#[cfg(feature = "inhibit")]
/// Get GTK's Wayland surface ID for use with idle inhibit.
/// Must be called from the GTK main thread.
pub fn get_gtk_wayland_surface_id(
    app: &gtk::Application,
) -> color_eyre::Result<wayland_client::backend::ObjectId> {
    use gdk4_wayland::WaylandSurface;
    use gdk4_wayland::prelude::WaylandSurfaceExtManual;
    use gtk::prelude::*;

    let windows = app.windows();
    let window = windows
        .first()
        .ok_or_else(|| color_eyre::eyre::eyre!("No windows available"))?;

    let gdk_surface = window
        .surface()
        .ok_or_else(|| color_eyre::eyre::eyre!("Window has no surface"))?;

    let wayland_surface = gdk_surface
        .downcast::<WaylandSurface>()
        .map_err(|_| color_eyre::eyre::eyre!("Not a Wayland surface"))?;

    let wl_surface = wayland_surface
        .wl_surface()
        .ok_or_else(|| color_eyre::eyre::eyre!("Failed to get wl_surface"))?;

    Ok(wl_surface.id())
}

#[derive(Debug)]
pub struct Environment {
    conn: Connection,
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,

    queue_handle: QueueHandle<Self>,

    event_tx: mpsc::Sender<Event>,
    response_tx: std::sync::mpsc::Sender<Response>,

    // local state
    #[cfg(any(feature = "focused", feature = "launcher"))]
    handles: Vec<ToplevelHandle>,

    // -- clipboard --
    #[cfg(feature = "clipboard")]
    data_control_device_manager_state: Option<DataControlDeviceManagerState>,

    #[cfg(feature = "clipboard")]
    data_control_devices: Vec<DataControlDeviceEntry>,
    #[cfg(feature = "clipboard")]
    copy_paste_sources: Vec<CopyPasteSource>,

    // local state
    #[cfg(feature = "clipboard")]
    clipboard: Arc<Mutex<Option<ClipboardItem>>>,

    // -- inhibit --
    #[cfg(feature = "inhibit")]
    inhibit_state: Option<InhibitState>,
}

delegate_registry!(Environment);

delegate_output!(Environment);
delegate_seat!(Environment);

cfg_if! {
    if #[cfg(any(feature = "focused", feature = "launcher"))] {
        delegate_foreign_toplevel_manager!(Environment);
        delegate_foreign_toplevel_handle!(Environment);
    }
}

cfg_if! {
    if #[cfg(feature = "clipboard")] {
        delegate_data_control_device_manager!(Environment);
        delegate_data_control_device!(Environment);
        delegate_data_control_offer!(Environment);
        delegate_data_control_source!(Environment);
    }
}

cfg_if! {
    if #[cfg(feature = "inhibit")] {
        use wayland_client::delegate_noop;

        delegate_noop!(Environment: ZwpIdleInhibitManagerV1);
        delegate_noop!(Environment: ZwpIdleInhibitorV1);
    }
}

impl Environment {
    pub fn spawn(
        conn: Connection,
        event_tx: mpsc::Sender<Event>,
        request_rx: calloop_channel::Channel<Request>,
        response_tx: std::sync::mpsc::Sender<Response>,
    ) {
        let (globals, queue) =
            registry_queue_init(&conn).expect("Failed to retrieve Wayland globals");

        let qh = queue.handle();
        let mut event_loop = EventLoop::<Self>::try_new().expect("Failed to create new event loop");

        WaylandSource::new(conn.clone(), queue)
            .insert(event_loop.handle())
            .expect("Failed to insert Wayland event queue into event loop");

        let loop_handle = event_loop.handle();

        // Initialize the registry handling
        // so other parts of Smithay's client toolkit may bind globals.
        let registry_state = RegistryState::new(&globals);

        let output_state = OutputState::new(&globals, &qh);
        let seat_state = SeatState::new(&globals, &qh);
        #[cfg(any(feature = "focused", feature = "launcher"))]
        if let Err(err) = ToplevelManagerState::bind(&globals, &qh) {
            error!("{:?}",
                Report::new(err)
                    .wrap_err("Failed to bind to wlr_foreign_toplevel_manager global")
                    .note("This is likely a due to the current compositor not supporting the required protocol")
                    .note("launcher and focused modules will not work")
            );
        }

        #[cfg(feature = "clipboard")]
        let data_control_device_manager_state = match DataControlDeviceManagerState::bind(
            &globals, &qh,
        ) {
            Ok(state) => Some(state),
            Err(err) => {
                error!("{:?}",
                    Report::new(err)
                        .wrap_err("Failed to bind to wlr_data_control_device global")
                        .note("This is likely a due to the current compositor not supporting the required protocol")
                        .note("clipboard module will not work")
                    );
                None
            }
        };

        let mut env = Self {
            conn: conn.clone(),
            registry_state,
            output_state,
            seat_state,
            #[cfg(feature = "clipboard")]
            data_control_device_manager_state,
            queue_handle: qh,
            event_tx,
            response_tx,
            #[cfg(any(feature = "focused", feature = "launcher"))]
            handles: vec![],

            #[cfg(feature = "clipboard")]
            data_control_devices: vec![],
            #[cfg(feature = "clipboard")]
            copy_paste_sources: vec![],
            #[cfg(feature = "clipboard")]
            clipboard: arc_mut!(None),

            #[cfg(feature = "inhibit")]
            inhibit_state: None,
        };

        loop_handle
            .insert_source(request_rx, Self::on_request)
            .expect("to be able to insert source");

        loop {
            trace!("Dispatching event loop");
            if let Err(err) = event_loop.dispatch(None, &mut env) {
                error!(
                    "{:?}",
                    Report::new(err).wrap_err("Failed to dispatch pending wayland events")
                );

                exit(ExitCode::WaylandDispatchError as i32)
            }
        }
    }

    /// Processes a request from the client
    /// and sends the response.
    fn on_request(event: calloop_channel::Event<Request>, _metadata: &mut (), env: &mut Self) {
        trace!("Request: {event:?}");

        match event {
            Msg(Request::Roundtrip) => {
                debug!("received roundtrip request");
                env.response_tx.send_expect(Response::Ok);
            }
            #[cfg(feature = "ipc")]
            Msg(Request::OutputInfoAll) => {
                let infos = env.output_info_all();
                env.response_tx.send_expect(Response::OutputInfoAll(infos));
            }
            #[cfg(any(feature = "focused", feature = "launcher"))]
            Msg(Request::ToplevelInfoAll) => {
                let infos = env
                    .handles
                    .iter()
                    .filter_map(ToplevelHandle::info)
                    .collect();

                env.response_tx
                    .send_expect(Response::ToplevelInfoAll(infos));
            }
            #[cfg(feature = "launcher")]
            Msg(Request::ToplevelFocus(id)) => {
                let handle = env
                    .handles
                    .iter()
                    .find(|handle| handle.info().is_some_and(|info| info.id == id));

                if let Some(handle) = handle {
                    let seat = env.default_seat();
                    handle.focus(&seat);
                }

                env.response_tx.send_expect(Response::Ok);
            }
            #[cfg(feature = "launcher")]
            Msg(Request::ToplevelMinimize(id)) => {
                let handle = env
                    .handles
                    .iter()
                    .find(|handle| handle.info().is_some_and(|info| info.id == id));

                if let Some(handle) = handle {
                    handle.minimize();
                }

                env.response_tx.send_expect(Response::Ok);
            }
            #[cfg(feature = "clipboard")]
            Msg(Request::CopyToClipboard(item)) => {
                env.copy_to_clipboard(item);
                env.response_tx.send_expect(Response::Ok);
            }
            #[cfg(feature = "clipboard")]
            Msg(Request::ClipboardItem) => {
                let item = lock!(env.clipboard).clone();
                env.response_tx.send_expect(Response::ClipboardItem(item));
            }

            #[cfg(feature = "inhibit")]
            Msg(Request::StartInhibit(duration, surface_id)) => {
                use crate::clients::inhibit::calculate_expiry;

                debug!(
                    "StartInhibit received: duration={:?}, surface_id={:?}",
                    duration, surface_id
                );

                // Initialize inhibit state on first call
                if env.inhibit_state.is_none() {
                    debug!("Initializing inhibit state (first call)");
                    match env
                        .registry_state
                        .bind_one::<ZwpIdleInhibitManagerV1, _, _>(&env.queue_handle, 1..=1, ())
                    {
                        Ok(manager) => {
                            debug!("Successfully bound idle inhibit manager");
                            env.inhibit_state = Some(InhibitState {
                                manager,
                                inhibitor: None,
                                expiry: None,
                            });
                        }
                        Err(err) => {
                            error!("Failed to bind idle inhibit manager: {:?}", err);
                            env.response_tx.send_expect(Response::Ok);
                            return;
                        }
                    }
                }

                if let Some(state) = &mut env.inhibit_state {
                    debug!("Inhibit state exists, updating...");

                    // Destroy existing inhibitor if any
                    if let Some(inhibitor) = state.inhibitor.take() {
                        debug!("Destroying existing inhibitor");
                        inhibitor.destroy();
                    }

                    // Create WlSurface proxy from the ObjectId
                    let surface = match WlSurface::from_id(&env.conn, surface_id) {
                        Ok(surface) => surface,
                        Err(err) => {
                            error!("Failed to create surface proxy from id: {:?}", err);
                            env.response_tx.send_expect(Response::Ok);
                            return;
                        }
                    };

                    // Create new inhibitor
                    debug!("Creating new inhibitor on surface {:?}", surface.id());
                    let inhibitor = state
                        .manager
                        .create_inhibitor(&surface, &env.queue_handle, ());
                    debug!("Inhibitor created successfully");
                    state.inhibitor = Some(inhibitor);
                    state.expiry = calculate_expiry(duration);
                    debug!("Expiry set to: {:?}", state.expiry);
                }

                debug!("StartInhibit completed, sending response");
                env.response_tx.send_expect(Response::Ok);
            }

            #[cfg(feature = "inhibit")]
            Msg(Request::StopInhibit) => {
                debug!("StopInhibit received");
                if let Some(state) = &mut env.inhibit_state {
                    if let Some(inhibitor) = state.inhibitor.take() {
                        debug!("Destroying inhibitor");
                        inhibitor.destroy();
                    }
                    state.expiry = None;
                    debug!("Inhibit stopped");
                } else {
                    debug!("No inhibit state to stop");
                }
                env.response_tx.send_expect(Response::Ok);
            }

            #[cfg(feature = "inhibit")]
            Msg(Request::GetInhibitExpiry) => {
                let expiry = env.inhibit_state.as_ref().and_then(|s| s.expiry);
                env.response_tx.send_expect(Response::InhibitExpiry(expiry));
            }

            calloop_channel::Event::Closed => error!("request channel unexpectedly closed"),
        }
    }
}

impl ProvidesRegistryState for Environment {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

register_client!(Client, wayland);
