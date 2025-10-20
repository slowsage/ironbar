use chrono::{DateTime, Utc};
use color_eyre::Result;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

/// Configuration enum for selecting inhibit backend.
///
/// **Valid options**: `systemd`, `wayland`
/// <br>
/// **Default**: `wayland`
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    /// Use systemd-inhibit via DBus
    Systemd,
    /// Use Wayland idle-inhibit protocol
    #[default]
    Wayland,
}

/// Runtime client for inhibiting idle/sleep behavior.
pub enum InhibitClient {
    Systemd(Arc<crate::clients::systemd_idle_inhibit::Client>),
    Wayland {
        client: Arc<crate::clients::wayland::Client>,
    },
}

impl InhibitClient {
    pub fn new(
        backend_type: BackendType,
        clients: &mut crate::clients::Clients,
    ) -> Result<Self> {
        match backend_type {
            BackendType::Systemd => {
                let client = clients.systemd_idle_inhibit()?;
                Ok(Self::Systemd(client))
            }
            BackendType::Wayland => {
                let client = clients.wayland();
                Ok(Self::Wayland { client })
            }
        }
    }

    pub fn expiry(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Systemd(client) => client.inhibit_expiry(),
            Self::Wayland { client } => client.inhibit_expiry(),
        }
    }

    pub async fn start(&mut self, duration: Duration, surface: wayland_client::backend::ObjectId) -> Result<()> {
        match self {
            Self::Systemd(client) => client.start(duration).await,
            Self::Wayland { client } => {
                client.start_inhibit(duration, surface);
                Ok(())
            }
        }
    }

    pub async fn stop(&mut self) -> Result<()> {
        match self {
            Self::Systemd(client) => client.stop().await,
            Self::Wayland { client } => {
                client.stop_inhibit();
                Ok(())
            }
        }
    }
}

pub(super) fn calculate_expiry(duration: Duration) -> Option<DateTime<Utc>> {
    match duration {
        // Map Duration::MAX to DateTime::MAX for infinite inhibit
        Duration::MAX => Some(DateTime::<Utc>::MAX_UTC),
        d => Utc::now().checked_add_signed(chrono::Duration::from_std(d).ok()?),
    }
}
