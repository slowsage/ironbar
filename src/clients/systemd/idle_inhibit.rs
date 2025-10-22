use super::Client;
use chrono::{DateTime, Utc};
use color_eyre::Result;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::zvariant::{OwnedObjectPath, Value};

const UNIT_NAME: &str = "ironbar-inhibit.service";
const CMD: &str = "/usr/bin/systemd-inhibit";
const SYSTEMD_SERVICE_INTERFACE: &str = "org.freedesktop.systemd1.Service";

/// Calculate expiry timestamp from duration.
/// Maps Duration::MAX to DateTime::MAX_UTC for infinite inhibit.
fn calculate_expiry(duration: Duration) -> Option<DateTime<Utc>> {
    match duration {
        Duration::MAX => Some(DateTime::<Utc>::MAX_UTC),
        d => Utc::now().checked_add_signed(chrono::Duration::from_std(d).ok()?),
    }
}

fn exec_tuple(duration: Duration) -> (String, Vec<String>, bool) {
    let sleep_arg = if duration == Duration::MAX {
        "infinity"
    } else {
        &duration.as_secs().to_string()
    };
    (
        CMD.to_string(),
        vec![
            CMD.to_string(),
            "--what=sleep:idle".into(),
            "--who=ironbar".into(),
            "--why=User requested".into(),
            "sleep".into(),
            sleep_arg.to_string(),
        ],
        false,
    )
}

fn read_env(env: &[String], key: &str) -> Option<u64> {
    env.iter()
        .find_map(|s| s.strip_prefix(&format!("{key}="))?.parse().ok())
}

#[derive(Debug)]
struct State {
    unit_path: Option<OwnedObjectPath>,
    expiry: Option<DateTime<Utc>>,
}

/// Manager for systemd-based idle inhibit.
#[derive(Debug, Clone)]
pub struct IdleInhibitManager {
    client: Arc<Client>,
    state: Arc<Mutex<State>>,
}

impl IdleInhibitManager {
    pub async fn new(client: Arc<Client>) -> Result<Self> {
        let unit_path = client.get_unit(UNIT_NAME).await.ok();
        let expiry = Self::restore_expiry(&client, &unit_path).await;

        Ok(Self {
            client,
            state: Arc::new(Mutex::new(State { unit_path, expiry })),
        })
    }

    async fn restore_expiry(
        client: &Client,
        unit_path: &Option<OwnedObjectPath>,
    ) -> Option<DateTime<Utc>> {
        let path = unit_path.as_ref()?;
        let env_value = client
            .get_property(path, SYSTEMD_SERVICE_INTERFACE, "Environment")
            .await
            .ok()?;
        let env = <Vec<String>>::try_from(env_value).ok()?;
        read_env(&env, "INHIBIT_EXPIRY").and_then(|ts| {
            let last_timestamp = ts as i64;
            if last_timestamp == i64::MAX {
                Some(DateTime::<Utc>::MAX_UTC)
            } else {
                DateTime::from_timestamp(last_timestamp, 0)
            }
        })
    }

    /// Get the current inhibit expiry time, if any.
    pub fn inhibit_expiry(&self) -> Option<DateTime<Utc>> {
        self.state
            .lock()
            .expect("Failed to lock state mutex")
            .expiry
    }

    /// Start inhibiting with the given duration.
    pub async fn start_inhibit(&self, duration: Duration) -> Result<()> {
        self.stop_inhibit().await?;

        let expiry = calculate_expiry(duration);
        let expiry_timestamp = match expiry {
            Some(timestamp) if timestamp == DateTime::<Utc>::MAX_UTC => i64::MAX,
            Some(timestamp) => timestamp.timestamp(),
            None => unreachable!(),
        };

        // Clone client before async operation to avoid holding lock across await
        let client = self.client.clone();
        {
            let mut state = self.state.lock().expect("Failed to lock state mutex");
            state.expiry = expiry;
        }

        let unit_path = client
            .start_transient_unit(
                UNIT_NAME,
                "replace",
                vec![
                    ("Description", Value::new("Ironbar Inhibit").try_to_owned()?),
                    ("Type", Value::new("simple").try_to_owned()?),
                    (
                        "ExecStart",
                        Value::new(vec![exec_tuple(duration)]).try_to_owned()?,
                    ),
                    (
                        "Environment",
                        Value::new(vec![format!("INHIBIT_EXPIRY={}", expiry_timestamp)])
                            .try_to_owned()?,
                    ),
                ],
                vec![],
            )
            .await?;

        let mut state = self.state.lock().expect("Failed to lock state mutex");
        state.unit_path = Some(unit_path);
        Ok(())
    }

    /// Stop inhibiting.
    pub async fn stop_inhibit(&self) -> Result<()> {
        // Clone client before async operation to avoid holding lock across await
        let (client, has_unit) = {
            let state = self.state.lock().expect("Failed to lock state mutex");
            (self.client.clone(), state.unit_path.is_some())
        };

        if has_unit {
            client.stop_unit(UNIT_NAME, "replace").await.ok();
        }

        let mut state = self.state.lock().expect("Failed to lock state mutex");
        state.unit_path = None;
        state.expiry = None;
        Ok(())
    }
}
