use crate::clients::{gtk_wayland, systemd};
use chrono::{DateTime, Utc};
use color_eyre::Result;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

/// Backend type for idle inhibit.
#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(feature = "extras", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    /// Systemd-based inhibit (persists across ironbar restarts).
    Systemd,
    /// Wayland protocol-based inhibit (tied to surface lifetime).
    #[default]
    Wayland,
}

/// Unified client for idle inhibit functionality.
#[derive(Debug, Clone)]
pub enum Client {
    Wayland(Arc<gtk_wayland::Client>),
    Systemd(Arc<systemd::IdleInhibitManager>),
}

impl Client {
    pub async fn new(
        backend: BackendType,
        wayland_client: Arc<gtk_wayland::Client>,
        systemd_client: Option<Arc<systemd::Client>>,
    ) -> Result<Self> {
        match backend {
            BackendType::Wayland => Ok(Self::Wayland(wayland_client)),
            BackendType::Systemd => {
                let systemd_client = systemd_client
                    .ok_or_else(|| color_eyre::eyre::eyre!("Systemd client not available"))?;
                let manager = systemd::IdleInhibitManager::new(systemd_client).await?;
                Ok(Self::Systemd(Arc::new(manager)))
            }
        }
    }

    /// Get the current inhibit expiry time, if any.
    pub fn inhibit_expiry(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Wayland(client) => client.idle_inhibit().inhibit_expiry(),
            Self::Systemd(manager) => manager.inhibit_expiry(),
        }
    }

    /// Start inhibiting with the given duration.
    pub async fn start_inhibit(&self, duration: Duration) -> Result<()> {
        match self {
            Self::Wayland(client) => {
                client.idle_inhibit().set_idle_inhibit(true, duration)?;
                Ok(())
            }
            Self::Systemd(manager) => manager.start_inhibit(duration).await,
        }
    }

    /// Stop inhibiting.
    pub async fn stop_inhibit(&self) -> Result<()> {
        match self {
            Self::Wayland(client) => client.idle_inhibit().stop(),
            Self::Systemd(manager) => manager.stop_inhibit().await,
        }
    }

    /// Get the backend type.
    pub fn backend_type(&self) -> BackendType {
        match self {
            Self::Wayland(_) => BackendType::Wayland,
            Self::Systemd(_) => BackendType::Systemd,
        }
    }
}
