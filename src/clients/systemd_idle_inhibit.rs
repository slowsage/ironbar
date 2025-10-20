use crate::register_fallible_client;
use chrono::{DateTime, Utc};
use color_eyre::Result;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue, Value},
    Connection,
};

const UNIT_NAME: &str = "ironbar-inhibit.service";
const CMD: &str = "/usr/bin/systemd-inhibit";
const SYSTEMD_SERVICE_INTERFACE: &str = "org.freedesktop.systemd1.Service";

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

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait SystemdManager {
    fn start_transient_unit(
        &self,
        name: &str,
        mode: &str,
        properties: Vec<(&str, OwnedValue)>,
        aux: Vec<(&str, Vec<(&str, OwnedValue)>)>,
    ) -> zbus::Result<OwnedObjectPath>;
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<()>;
    fn get_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.DBus.Properties",
    default_service = "org.freedesktop.systemd1"
)]
trait Properties {
    fn get(&self, interface: &str, property: &str) -> zbus::Result<OwnedValue>;
}

#[derive(Debug)]
struct State {
    proxy: SystemdManagerProxy<'static>,
    unit_path: Option<OwnedObjectPath>,
    expiry: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct Client {
    state: Arc<Mutex<State>>,
}

impl Client {
    pub(crate) async fn new() -> Result<Self> {
        let conn = Connection::session().await?;
        let proxy = SystemdManagerProxy::new(&conn).await?;
        let unit_path = proxy.get_unit(UNIT_NAME).await.ok();

        // Restore expiry from existing unit's Environment to survive ironbar restarts
        let expiry = Self::restore_expiry(&conn, &unit_path).await;

        Ok(Self {
            state: Arc::new(Mutex::new(State {
                proxy,
                unit_path,
                expiry,
            })),
        })
    }

    async fn restore_expiry(
        conn: &Connection,
        unit_path: &Option<OwnedObjectPath>,
    ) -> Option<DateTime<Utc>> {
        let path = unit_path.as_ref()?;
        let props = PropertiesProxy::builder(conn)
            .path(path)
            .ok()?
            .build()
            .await
            .ok()?;
        let env_value = props
            .get(SYSTEMD_SERVICE_INTERFACE, "Environment")
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

    pub fn inhibit_expiry(&self) -> Option<DateTime<Utc>> {
        self.state.lock().unwrap().expiry
    }

    pub async fn start(&self, duration: Duration) -> Result<()> {
        self.stop().await?;

        let expiry = calculate_expiry(duration);
        let expiry_timestamp = match expiry {
            Some(timestamp) if timestamp == DateTime::<Utc>::MAX_UTC => i64::MAX,
            Some(timestamp) => timestamp.timestamp(),
            None => unreachable!(),
        };

        // Clone proxy before async operation to avoid holding lock across await
        let proxy = {
            let mut state = self.state.lock().unwrap();
            state.expiry = expiry;
            state.proxy.clone()
        };

        let unit_path = proxy
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

        let mut state = self.state.lock().unwrap();
        state.unit_path = Some(unit_path);
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        // Clone proxy before async operation to avoid holding lock across await
        let (proxy, has_unit) = {
            let state = self.state.lock().unwrap();
            (state.proxy.clone(), state.unit_path.is_some())
        };

        if has_unit {
            proxy.stop_unit(UNIT_NAME, "replace").await.ok();
        }

        let mut state = self.state.lock().unwrap();
        state.unit_path = None;
        state.expiry = None;
        Ok(())
    }
}

fn calculate_expiry(duration: Duration) -> Option<DateTime<Utc>> {
    match duration {
        Duration::MAX => Some(DateTime::<Utc>::MAX_UTC),
        d => Utc::now().checked_add_signed(chrono::Duration::from_std(d).ok()?),
    }
}

register_fallible_client!(Client, systemd_idle_inhibit);
