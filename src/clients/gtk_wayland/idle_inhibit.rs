use chrono::{DateTime, Utc};
use gdk4_wayland::prelude::*;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, error, warn};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1;

/// Calculate expiry timestamp from duration.
/// Maps Duration::MAX to DateTime::MAX_UTC for infinite inhibit.
fn calculate_expiry(duration: Duration) -> Option<DateTime<Utc>> {
    match duration {
        Duration::MAX => Some(DateTime::<Utc>::MAX_UTC),
        d => Utc::now().checked_add_signed(chrono::Duration::from_std(d).ok()?),
    }
}

#[derive(Debug)]
struct AppState;

impl Dispatch<ZwpIdleInhibitorV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpIdleInhibitorV1,
        _event: <ZwpIdleInhibitorV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // No events to handle for idle inhibitor
    }
}

impl Dispatch<ZwpIdleInhibitManagerV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpIdleInhibitManagerV1,
        _event: <ZwpIdleInhibitManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // No events to handle for idle inhibit manager
    }
}

struct InhibitState {
    manager: ZwpIdleInhibitManagerV1,
    inhibitor: Option<ZwpIdleInhibitorV1>,
    expiry: Option<DateTime<Utc>>,
    surface: Option<WlSurface>,
    event_queue: EventQueue<AppState>,
}

// Custom Debug implementation required because EventQueue<AppState> doesn't implement Debug.
// We manually format the fields we care about and use placeholders for the opaque types.
impl std::fmt::Debug for InhibitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InhibitState")
            .field("manager", &"<ZwpIdleInhibitManagerV1>")
            .field("inhibitor", &self.inhibitor)
            .field("expiry", &self.expiry)
            .field("surface", &self.surface)
            .field("event_queue", &"<EventQueue>")
            .finish()
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

    /// Initialize the manager with a surface to use for inhibiting.
    /// This should be called once from the GTK main thread.
    /// Subsequent calls are no-ops.
    pub fn init_surface(&self, surface: WlSurface) {
        use std::sync::{Arc, Mutex};
        use wayland_client::protocol::wl_registry::{self, WlRegistry};

        let mut state = self.state.lock().unwrap();

        if state.is_some() {
            debug!("IdleInhibitManager already initialized, ignoring");
            return;
        }

        debug!("Initializing IdleInhibitManager with surface");

        // Get GTK's Wayland connection
        let display = gtk::gdk::Display::default().expect("GTK Display not initialized");
        let wl_display_gdk = display
            .downcast::<gdk4_wayland::WaylandDisplay>()
            .expect("Not a Wayland display");
        let wl_display = wl_display_gdk
            .wl_display()
            .expect("Failed to get wl_display");
        let backend = wl_display.backend().upgrade().expect("Backend destroyed");
        let conn = Connection::from_backend(backend);

        // Bind the idle inhibit manager protocol
        // We need a temporary state to capture the manager during registry events
        struct BindState {
            manager: Arc<Mutex<Option<ZwpIdleInhibitManagerV1>>>,
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
                {
                    if interface == "zwp_idle_inhibit_manager_v1" {
                        let manager: ZwpIdleInhibitManagerV1 =
                            registry.bind(name, version.min(1), qh, ());
                        *state.manager.lock().unwrap() = Some(manager);
                    }
                }
            }
        }

        impl Dispatch<ZwpIdleInhibitManagerV1, ()> for BindState {
            fn event(
                _state: &mut Self,
                _proxy: &ZwpIdleInhibitManagerV1,
                _event: <ZwpIdleInhibitManagerV1 as wayland_client::Proxy>::Event,
                _data: &(),
                _conn: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
            }
        }

        // Create temporary event queue for binding
        let mut bind_queue = conn.new_event_queue();
        let bind_qh = bind_queue.handle();

        let manager_arc = Arc::new(Mutex::new(None));
        let mut bind_state = BindState {
            manager: manager_arc.clone(),
        };

        // Get the registry and request globals
        let display_proxy = conn.display();
        let _registry = display_proxy.get_registry(&bind_qh, ());

        // Roundtrip to receive global events and bind the manager
        if let Err(e) = bind_queue.roundtrip(&mut bind_state) {
            error!("Failed to roundtrip during binding: {}", e);
            return;
        }

        // Get the bound manager
        let manager_opt = manager_arc.lock().unwrap().take();
        let Some(manager) = manager_opt else {
            error!("Failed to bind idle inhibit manager - protocol not supported");
            return;
        };

        debug!("Successfully bound idle inhibit manager");

        // Create the final event queue for AppState (used for inhibitor events)
        let event_queue = conn.new_event_queue();

        *state = Some(InhibitState {
            manager,
            inhibitor: None,
            expiry: None,
            surface: Some(surface),
            event_queue,
        });
    }

    /// Set idle inhibit state.
    /// If enabled=true, inhibits idle with the given duration.
    /// If enabled=false, stops inhibiting.
    pub fn set_idle_inhibit(&self, enabled: bool, duration: Duration) -> color_eyre::Result<()> {
        let mut state = self.state.lock().unwrap();

        let Some(ref mut st) = *state else {
            warn!("IdleInhibitManager not initialized - surface not set yet");
            return Ok(());
        };

        if enabled {
            // Destroy existing inhibitor if any
            if let Some(inhibitor) = st.inhibitor.take() {
                debug!("Destroying existing inhibitor");
                inhibitor.destroy();
            }

            let Some(ref surface) = st.surface else {
                error!("No surface available for inhibit");
                return Ok(());
            };

            // Create new inhibitor
            debug!("Creating new inhibitor on surface {:?}", surface.id());
            let qh = st.event_queue.handle();
            let inhibitor = st.manager.create_inhibitor(surface, &qh, ());

            debug!("Inhibitor created successfully");
            st.inhibitor = Some(inhibitor);
            st.expiry = calculate_expiry(duration);
            debug!("Expiry set to: {:?}", st.expiry);
        } else {
            // Stop inhibiting
            debug!("Stopping inhibit");
            if let Some(inhibitor) = st.inhibitor.take() {
                debug!("Destroying inhibitor");
                inhibitor.destroy();
            }
            st.expiry = None;
        }

        // Dispatch any pending events
        if let Err(e) = st.event_queue.dispatch_pending(&mut AppState) {
            warn!("Failed to dispatch pending events: {}", e);
        }

        Ok(())
    }

    /// Stop inhibiting (convenience method).
    pub fn stop(&self) -> color_eyre::Result<()> {
        self.set_idle_inhibit(false, Duration::ZERO)
    }

    /// Get the current inhibit expiry time, if any.
    pub fn inhibit_expiry(&self) -> Option<DateTime<Utc>> {
        let state = self.state.lock().unwrap();
        state.as_ref().and_then(|s| s.expiry)
    }
}
