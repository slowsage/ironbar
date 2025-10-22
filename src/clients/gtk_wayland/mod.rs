mod idle_inhibit;

use crate::register_client;

pub use idle_inhibit::IdleInhibitManager;

/// Client wrapping GTK's Wayland connection and providing access
/// to protocol managers that need to share GTK's connection.
#[derive(Debug)]
pub struct Client {
    idle_inhibit: IdleInhibitManager,
}

impl Client {
    pub fn new() -> Self {
        let idle_inhibit = IdleInhibitManager::new();

        Self { idle_inhibit }
    }

    /// Get the idle inhibit manager for controlling system idle inhibition.
    pub fn idle_inhibit(&self) -> &IdleInhibitManager {
        &self.idle_inhibit
    }
}

register_client!(Client, gtk_wayland);
