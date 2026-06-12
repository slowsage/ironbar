#[cfg(feature = "bindmode+hyprland")]
use super::{BindModeClient, BindModeUpdate};
#[cfg(feature = "keyboard+hyprland")]
use super::{KeyboardLayoutClient, KeyboardLayoutUpdate};
#[cfg(feature = "workspaces+hyprland")]
use super::{Visibility, Workspace, WorkspaceUpdate};
use crate::channels::SyncSenderExt;
use crate::spawn_blocking;
use color_eyre::Result;
use serde::Deserialize;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::broadcast::{Receiver, Sender, channel};
use tracing::{debug, error, info};

/// Delay before reconnecting to the event socket after it closes.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug)]
struct TxRx<T> {
    tx: Sender<T>,
    _rx: Receiver<T>,
}
impl<T: Clone> TxRx<T> {
    fn new() -> Self {
        let (tx, rx) = channel(16);
        Self { tx, _rx: rx }
    }

    fn subscribe(&self) -> Receiver<T> {
        self.tx.subscribe()
    }
}

impl<T: std::fmt::Debug> TxRx<T> {
    fn send(&self, update: T) {
        self.tx.send_expect(update);
    }
}

#[derive(Debug)]
pub struct Client {
    #[cfg(feature = "workspaces+hyprland")]
    workspace: TxRx<WorkspaceUpdate>,

    #[cfg(feature = "keyboard+hyprland")]
    keyboard_layout: TxRx<KeyboardLayoutUpdate>,

    #[cfg(feature = "bindmode+hyprland")]
    bindmode: TxRx<BindModeUpdate>,
}

impl Client {
    pub(crate) fn new() -> Self {
        let instance = Self {
            #[cfg(feature = "workspaces+hyprland")]
            workspace: TxRx::new(),
            #[cfg(feature = "keyboard+hyprland")]
            keyboard_layout: TxRx::new(),
            #[cfg(feature = "bindmode+hyprland")]
            bindmode: TxRx::new(),
        };

        instance.listen_events();
        instance
    }

    /// Reads the Hyprland event socket (`.socket2.sock`) and forwards each
    /// event to the relevant module channel.
    fn listen_events(&self) {
        info!("Starting Hyprland event listener");

        #[cfg(feature = "workspaces+hyprland")]
        let workspace_tx = self.workspace.tx.clone();
        #[cfg(feature = "keyboard+hyprland")]
        let keyboard_layout_tx = self.keyboard_layout.tx.clone();
        #[cfg(feature = "bindmode+hyprland")]
        let bindmode_tx = self.bindmode.tx.clone();

        std::thread::spawn(move || {
            loop {
                let stream = match UnixStream::connect(socket_path(".socket2.sock")) {
                    Ok(stream) => stream,
                    Err(err) => {
                        error!("Failed to connect to Hyprland event socket: {err:#}");
                        std::thread::sleep(RECONNECT_DELAY);
                        continue;
                    }
                };

                // cache the active workspace since Hyprland doesn't give us the prev active.
                // re-fetched on each (re)connection so it can't go stale across a restart.
                #[cfg(feature = "workspaces+hyprland")]
                let mut active = Self::get_active_workspace()
                    .map_err(|err| error!("Failed to get active workspace: {err:#}"))
                    .ok();

                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line.map_err(|err| error!("Event socket read failed: {err:#}"))
                    else {
                        break;
                    };
                    let Some((event, data)) = line.split_once(">>") else {
                        continue;
                    };
                    debug!("Received event: {event} >> {data}");

                    match event {
                        #[cfg(feature = "workspaces+hyprland")]
                        _ if Self::handle_workspace_event(
                            event,
                            data,
                            &mut active,
                            &workspace_tx,
                        ) => {}
                        #[cfg(feature = "keyboard+hyprland")]
                        "activelayout" => keyboard_layout_tx
                            .send_expect(KeyboardLayoutUpdate(field(data, 1, 2).to_string())),
                        #[cfg(feature = "bindmode+hyprland")]
                        "submap" => bindmode_tx.send_expect(BindModeUpdate {
                            name: data.to_string(),
                            pango_markup: false,
                        }),
                        _ => {}
                    }
                }

                // the event socket closed (e.g. compositor restart) or a read failed;
                // reconnect after a short delay rather than dying permanently.
                error!("Hyprland event socket closed; reconnecting");
                std::thread::sleep(RECONNECT_DELAY);
            }
        });
    }
}

#[cfg(feature = "workspaces+hyprland")]
impl Client {
    /// Handles a single workspace event from the event socket, updating
    /// `active` and notifying `tx`. Returns `false` if `event` isn't one.
    fn handle_workspace_event(
        event: &str,
        data: &str,
        active: &mut Option<Workspace>,
        tx: &Sender<WorkspaceUpdate>,
    ) -> bool {
        match event {
            // both carry the newly focused workspace name in the last field
            "workspacev2" | "focusedmon" => {
                if let Some(ws) = Self::get_workspace(field(data, 1, 2), active.as_ref()) {
                    Self::send_focus_if_changed(active, ws, tx);
                }
            }
            "createworkspacev2" => {
                if let Some(ws) = Self::get_workspace(field(data, 1, 2), active.as_ref()) {
                    tx.send_expect(WorkspaceUpdate::Add(ws));
                }
            }
            "moveworkspacev2" => {
                if let Some(ws) = Self::get_workspace(field(data, 1, 3), active.as_ref()) {
                    tx.send_expect(WorkspaceUpdate::Move(ws.clone()));
                    Self::send_focus_if_changed(active, ws, tx);
                }
            }
            "renameworkspace" | "destroyworkspacev2" => {
                if let Ok(id) = field(data, 0, 2).parse() {
                    tx.send_expect(if event == "renameworkspace" {
                        WorkspaceUpdate::Rename {
                            id,
                            name: field(data, 1, 2).to_string(),
                        }
                    } else {
                        WorkspaceUpdate::Remove(id)
                    });
                }
            }
            "urgent" => Self::send_urgent(&format!("0x{data}"), tx),
            _ => return false,
        }
        true
    }

    /// Sends a `WorkspaceUpdate::Focus` event
    /// and updates the active workspace cache.
    fn send_focus_if_changed(
        prev_workspace: &mut Option<Workspace>,
        workspace: Workspace,
        tx: &Sender<WorkspaceUpdate>,
    ) {
        if !workspace.visibility.is_focused() {
            Self::send_focus_change(prev_workspace, workspace, tx);
        }
    }

    fn send_focus_change(
        prev_workspace: &mut Option<Workspace>,
        workspace: Workspace,
        tx: &Sender<WorkspaceUpdate>,
    ) {
        tx.send_expect(WorkspaceUpdate::Focus {
            old: prev_workspace.take(),
            new: workspace.clone(),
        });
        tx.send_expect(WorkspaceUpdate::Urgent {
            id: workspace.id,
            urgent: false,
        });
        prev_workspace.replace(workspace);
    }

    /// Resolves the window `address` to its workspace and flags it urgent.
    fn send_urgent(address: &str, tx: &Sender<WorkspaceUpdate>) {
        match request_json::<Vec<HClient>>("j/clients") {
            Ok(clients) => match clients.into_iter().find(|c| c.address == address) {
                Some(client) => tx.send_expect(WorkspaceUpdate::Urgent {
                    id: client.workspace.id,
                    urgent: true,
                }),
                None => error!("Unable to locate client"),
            },
            Err(err) => error!("Failed to get clients: {err:#}"),
        }
    }

    /// Gets a workspace by name from the server, logging on failure.
    fn get_workspace(name: &str, active: Option<&Workspace>) -> Option<Workspace> {
        let workspaces = Self::get_workspaces(active.map(|w| w.name.as_str()))
            .map_err(|err| error!("Failed to get workspace: {err:#}"))
            .ok()?;
        let workspace = workspaces.into_iter().find(|w| w.name == name);
        if workspace.is_none() {
            error!("Unable to locate workspace");
        }
        workspace
    }

    /// Gets all workspaces from the server,
    /// with visibility computed against the active workspace name.
    fn get_workspaces(active: Option<&str>) -> Result<Vec<Workspace>> {
        let monitors = request_json::<Vec<HMonitor>>("j/monitors")?;

        let workspaces = request_json::<Vec<HWorkspace>>("j/workspaces")?
            .into_iter()
            .map(|w| {
                let visibility = if Some(w.name.as_str()) == active {
                    Visibility::focused()
                } else if monitors.iter().any(|m| m.active_workspace.id == w.id) {
                    Visibility::visible()
                } else {
                    Visibility::Hidden
                };
                Workspace::from((visibility, w))
            })
            .collect();

        Ok(workspaces)
    }

    fn get_active_workspace() -> Result<Workspace> {
        let workspace = request_json::<HWorkspace>("j/activeworkspace")?;
        Ok(Workspace::from((Visibility::focused(), workspace)))
    }
}

#[cfg(feature = "workspaces+hyprland")]
impl super::WorkspaceClient for Client {
    fn focus(&self, id: i64) {
        // Requires Hyprland's lua config (0.55+); the legacy `.conf` dispatch
        // syntax (`dispatch workspace <id>`) is not supported.
        let command = format!("dispatch hl.dsp.focus({{ workspace = \"{id}\" }})");
        spawn_blocking(move || match request(&command) {
            Ok(reply) if !reply.starts_with("ok") => {
                error!("dispatch '{command}' rejected: {reply}");
            }
            Err(err) => error!("dispatch '{command}' failed: {err:#}"),
            _ => {}
        });
    }

    fn subscribe(&self) -> Receiver<WorkspaceUpdate> {
        let rx = self.workspace.subscribe();

        let active = Self::get_active_workspace().ok().map(|w| w.name);
        match Self::get_workspaces(active.as_deref()) {
            Ok(workspaces) => self.workspace.send(WorkspaceUpdate::Init(workspaces)),
            Err(err) => error!("Failed to get workspaces: {err:#}"),
        }

        rx
    }
}

#[cfg(feature = "keyboard+hyprland")]
impl KeyboardLayoutClient for Client {
    fn set_next_active(&self) {
        spawn_blocking(|| {
            if let Some(keyboard) = main_keyboard()
                && let Err(err) = request(&format!("switchxkblayout {} next", keyboard.name))
            {
                error!("Failed to switch keyboard layout: {err:#}");
            }
        });
    }

    fn subscribe(&self) -> Receiver<KeyboardLayoutUpdate> {
        let rx = self.keyboard_layout.subscribe();

        if let Some(keyboard) = main_keyboard() {
            self.keyboard_layout
                .send(KeyboardLayoutUpdate(keyboard.active_keymap));
        }

        rx
    }
}

#[cfg(feature = "bindmode+hyprland")]
impl BindModeClient for Client {
    fn subscribe(&self) -> super::Result<Receiver<BindModeUpdate>> {
        Ok(self.bindmode.subscribe())
    }
}

/// Path to a Hyprland IPC socket for the current instance.
fn socket_path(name: &str) -> PathBuf {
    let signature = env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap_or_default();
    env::var("XDG_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join("hypr").join(&signature))
        .ok()
        .filter(|dir| dir.exists())
        .unwrap_or_else(|| PathBuf::from("/tmp/hypr").join(&signature))
        .join(name)
}

/// Sends a command over the request socket (`.socket.sock`) and returns the reply.
#[cfg(any(feature = "workspaces+hyprland", feature = "keyboard+hyprland"))]
fn request(command: &str) -> Result<String> {
    let mut stream = UnixStream::connect(socket_path(".socket.sock"))?;
    stream.write_all(command.as_bytes())?;
    stream.shutdown(Shutdown::Write)?;

    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

/// Sends a `j/`-prefixed command and deserializes the JSON reply.
#[cfg(any(feature = "workspaces+hyprland", feature = "keyboard+hyprland"))]
fn request_json<T: serde::de::DeserializeOwned>(command: &str) -> Result<T> {
    Ok(serde_json::from_str(&request(command)?)?)
}

/// Gets the `index`th of `count` comma-separated event fields,
/// keeping any extra commas in the final field.
#[cfg(any(feature = "workspaces+hyprland", feature = "keyboard+hyprland"))]
fn field(data: &str, index: usize, count: usize) -> &str {
    data.splitn(count, ',').nth(index).unwrap_or_default()
}

/// Gets the main keyboard from the server, logging on failure.
#[cfg(feature = "keyboard+hyprland")]
fn main_keyboard() -> Option<HKeyboard> {
    let devices = request_json::<HDevices>("j/devices")
        .map_err(|err| error!("Failed to get devices: {err:#}"))
        .ok()?;
    let keyboard = devices.keyboards.into_iter().find(|k| k.main);
    if keyboard.is_none() {
        error!("Failed to get keyboard device from hyprland");
    }
    keyboard
}

/// Minimal subset of Hyprland's `workspaces` JSON.
#[cfg(feature = "workspaces+hyprland")]
#[derive(Deserialize)]
struct HWorkspace {
    id: i64,
    name: String,
    monitor: String,
}

/// Minimal subset of Hyprland's `monitors` JSON.
#[cfg(feature = "workspaces+hyprland")]
#[derive(Deserialize)]
struct HMonitor {
    #[serde(rename = "activeWorkspace")]
    active_workspace: HWorkspaceRef,
}

/// Minimal subset of Hyprland's `clients` JSON.
#[cfg(feature = "workspaces+hyprland")]
#[derive(Deserialize)]
struct HClient {
    address: String,
    workspace: HWorkspaceRef,
}

#[cfg(feature = "workspaces+hyprland")]
#[derive(Deserialize)]
struct HWorkspaceRef {
    id: i64,
}

/// Minimal subset of Hyprland's `devices` JSON.
#[cfg(feature = "keyboard+hyprland")]
#[derive(Deserialize)]
struct HDevices {
    keyboards: Vec<HKeyboard>,
}

#[cfg(feature = "keyboard+hyprland")]
#[derive(Deserialize)]
struct HKeyboard {
    name: String,
    main: bool,
    active_keymap: String,
}

#[cfg(feature = "workspaces+hyprland")]
impl From<(Visibility, HWorkspace)> for Workspace {
    fn from((visibility, workspace): (Visibility, HWorkspace)) -> Self {
        Self {
            id: workspace.id,
            index: workspace.id,
            name: workspace.name,
            monitor: workspace.monitor,
            visibility,
        }
    }
}
