use crate::lock;
use chrono::{DateTime, Utc};
use gdk4_wayland::prelude::*;
use gtk::prelude::*;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, warn};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1;

fn calculate_expiry(duration: Duration) -> Option<DateTime<Utc>> {
    if duration == Duration::MAX {
        return Some(DateTime::<Utc>::MAX_UTC);
    }
    let chrono_duration = chrono::Duration::from_std(duration).ok()?;
    Utc::now().checked_add_signed(chrono_duration)
}

#[derive(Debug)]
struct AppState;

wayland_client::delegate_noop!(AppState: ignore ZwpIdleInhibitorV1);
wayland_client::delegate_noop!(AppState: ignore ZwpIdleInhibitManagerV1);

struct InhibitState {
    manager: ZwpIdleInhibitManagerV1,
    inhibitor: Option<ZwpIdleInhibitorV1>,
    expiry: Option<DateTime<Utc>>,
    surface: WlSurface,
    event_queue: EventQueue<AppState>,
}

impl std::fmt::Debug for InhibitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InhibitState")
            .field("expiry", &self.expiry)
            .finish_non_exhaustive()
    }
}

/// Manager for idle inhibit protocol on GTK's Wayland connection.
#[derive(Debug)]
pub struct IdleInhibitManager {
    state: Mutex<Option<InhibitState>>,
}

impl IdleInhibitManager {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(None),
        }
    }

    /// Initialize from GTK widget. Call once from GTK main thread.
    pub fn init_from_widget(&self, widget: &impl gtk::prelude::IsA<gtk::Widget>) {
        use gdk4_wayland::prelude::WaylandSurfaceExtManual;

        if let Some(surface) = widget
            .root()
            .and_then(|r| r.downcast::<gtk::Window>().ok())
            .and_then(|w| w.surface())
            .and_then(|s| s.downcast::<gdk4_wayland::WaylandSurface>().ok())
            .and_then(|ws| ws.wl_surface())
        {
            self.init_surface(surface);
        }
    }

    /// Initialize with surface. Call once from GTK main thread.
    fn init_surface(&self, surface: WlSurface) {
        use wayland_client::protocol::wl_registry::{self, WlRegistry};

        let mut state = lock!(self.state);
        if state.is_some() {
            return;
        }

        let Some(conn) = gtk::gdk::Display::default()
            .and_downcast::<gdk4_wayland::WaylandDisplay>()
            .and_then(|d| d.wl_display())
            .and_then(|d| d.backend().upgrade().map(Connection::from_backend))
        else {
            return;
        };

        struct BindState {
            manager: Option<ZwpIdleInhibitManagerV1>,
        }

        impl Dispatch<WlRegistry, ()> for BindState {
            fn event(
                state: &mut Self,
                registry: &WlRegistry,
                event: wl_registry::Event,
                _data: &(),
                _conn: &Connection,
                qh: &QueueHandle<Self>,
            ) {
                if let wl_registry::Event::Global {
                    name,
                    interface,
                    version,
                } = event
                    && interface == "zwp_idle_inhibit_manager_v1"
                {
                    state.manager = Some(registry.bind(name, version.min(1), qh, ()));
                }
            }
        }

        wayland_client::delegate_noop!(BindState: ignore ZwpIdleInhibitManagerV1);

        let mut bind_queue = conn.new_event_queue();
        let bind_qh = bind_queue.handle();
        let mut bind_state = BindState { manager: None };

        conn.display().get_registry(&bind_qh, ());

        if bind_queue.roundtrip(&mut bind_state).is_err() {
            return;
        }

        let Some(manager) = bind_state.manager else {
            return;
        };

        debug!("Bound idle inhibit manager");

        let event_queue = conn.new_event_queue();

        *state = Some(InhibitState {
            manager,
            inhibitor: None,
            expiry: None,
            surface,
            event_queue,
        });
    }

    fn set_idle_inhibit_impl(&self, enabled: bool, duration: Duration) {
        let mut state = lock!(self.state);

        let Some(ref mut st) = *state else {
            warn!("IdleInhibitManager not initialized - surface not set yet");
            return;
        };

        if let Some(inhibitor) = st.inhibitor.take() {
            inhibitor.destroy();
        }

        if enabled {
            st.inhibitor = Some(st.manager.create_inhibitor(
                &st.surface,
                &st.event_queue.handle(),
                (),
            ));
            st.expiry = calculate_expiry(duration);
        } else {
            st.expiry = None;
        }
    }

    pub fn start(&self, duration: Duration) {
        self.set_idle_inhibit_impl(true, duration)
    }

    pub fn stop(&self) {
        self.set_idle_inhibit_impl(false, Duration::ZERO)
    }

    pub fn expiry(&self) -> Option<DateTime<Utc>> {
        lock!(self.state).as_ref().and_then(|s| s.expiry)
    }
}
