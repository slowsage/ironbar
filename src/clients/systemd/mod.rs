mod idle_inhibit;

use color_eyre::Result;
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue},
    Connection,
};

pub use idle_inhibit::IdleInhibitManager;

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
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn get_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.DBus.Properties",
    default_service = "org.freedesktop.systemd1"
)]
trait Properties {
    fn get(&self, interface: &str, property: &str) -> zbus::Result<OwnedValue>;
}

/// General systemd client providing service management primitives.
#[derive(Debug, Clone)]
pub struct Client {
    manager: SystemdManagerProxy<'static>,
    conn: Connection,
}

impl Client {
    pub(crate) async fn new() -> Result<Self> {
        let conn = Connection::system().await?;
        let manager = SystemdManagerProxy::new(&conn).await?;
        Ok(Self { manager, conn })
    }

    /// Start a transient systemd unit.
    pub async fn start_transient_unit(
        &self,
        name: &str,
        mode: &str,
        properties: Vec<(&str, OwnedValue)>,
        aux: Vec<(&str, Vec<(&str, OwnedValue)>)>,
    ) -> Result<OwnedObjectPath> {
        Ok(self.manager.start_transient_unit(name, mode, properties, aux).await?)
    }

    /// Stop a systemd unit.
    pub async fn stop_unit(&self, name: &str, mode: &str) -> Result<OwnedObjectPath> {
        Ok(self.manager.stop_unit(name, mode).await?)
    }

    /// Get the object path for a unit.
    pub async fn get_unit(&self, name: &str) -> Result<OwnedObjectPath> {
        Ok(self.manager.get_unit(name).await?)
    }

    /// Get a property from a systemd object.
    pub async fn get_property(
        &self,
        path: &OwnedObjectPath,
        interface: &str,
        property: &str,
    ) -> Result<OwnedValue> {
        let props = PropertiesProxy::builder(&self.conn)
            .path(path)?
            .build()
            .await?;
        Ok(props.get(interface, property).await?)
    }

    /// Get the DBus connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}
