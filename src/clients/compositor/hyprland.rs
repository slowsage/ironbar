#[cfg(feature = "bindmode+hyprland")]
use super::{BindModeClient, BindModeUpdate};
#[cfg(feature = "keyboard+hyprland")]
use super::{KeyboardLayoutClient, KeyboardLayoutUpdate};
use super::{Visibility, Workspace};
use crate::channels::SyncSenderExt;
use crate::spawn_blocking;
use color_eyre::Result;
use serde::Deserialize;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use tokio::sync::broadcast::{Receiver, Sender, channel};
use tracing::{debug, error, info};

#[cfg(feature = "workspaces")]
use super::WorkspaceUpdate;

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

        spawn_blocking(move || {
            let stream = match UnixStream::connect(socket_path(".socket2.sock")) {
                Ok(stream) => stream,
                Err(err) => return error!("Failed to connect to Hyprland event socket: {err:#}"),
            };

            // cache the active workspace since Hyprland doesn't give us the prev active
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
                    // both carry the newly focused workspace name in the last field
                    #[cfg(feature = "workspaces+hyprland")]
                    "workspacev2" | "focusedmon" => {
                        if let Some(ws) = Self::get_workspace(field(data, 1, 2), active.as_ref())
                            && !ws.visibility.is_focused()
                        {
                            Self::send_focus_change(&mut active, ws, &workspace_tx);
                        }
                    }
                    #[cfg(feature = "workspaces+hyprland")]
                    "createworkspacev2" => {
                        if let Some(ws) = Self::get_workspace(field(data, 1, 2), active.as_ref()) {
                            workspace_tx.send_expect(WorkspaceUpdate::Add(ws));
                        }
                    }
                    #[cfg(feature = "workspaces+hyprland")]
                    "moveworkspacev2" => {
                        if let Some(ws) = Self::get_workspace(field(data, 1, 3), active.as_ref()) {
                            workspace_tx.send_expect(WorkspaceUpdate::Move(ws.clone()));
                            if !ws.visibility.is_focused() {
                                Self::send_focus_change(&mut active, ws, &workspace_tx);
                            }
                        }
                    }
                    #[cfg(feature = "workspaces+hyprland")]
                    "renameworkspace" | "destroyworkspacev2" => {
                        if let Ok(id) = field(data, 0, 2).parse() {
                            workspace_tx.send_expect(if event == "renameworkspace" {
                                WorkspaceUpdate::Rename {
                                    id,
                                    name: field(data, 1, 2).to_string(),
                                }
                            } else {
                                WorkspaceUpdate::Remove(id)
                            });
                        }
                    }
                    #[cfg(feature = "workspaces+hyprland")]
                    "urgent" => Self::send_urgent(&format!("0x{data}"), &workspace_tx),
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
        });
    }

    /// Sends a `WorkspaceUpdate::Focus` event
    /// and updates the active workspace cache.
    #[cfg(feature = "workspaces+hyprland")]
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
    #[cfg(feature = "workspaces+hyprland")]
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
    #[cfg(feature = "workspaces+hyprland")]
    fn get_workspace(name: &str, active: Option<&Workspace>) -> Option<Workspace> {
        match Self::get_workspaces(active.map(|w| w.name.as_str())) {
            Ok(workspaces) => {
                let workspace = workspaces.into_iter().find(|w| w.name == name);
                if workspace.is_none() {
                    error!("Unable to locate workspace");
                }
                workspace
            }
            Err(err) => {
                error!("Failed to get workspace: {err:#}");
                None
            }
        }
    }

    /// Gets all workspaces from the server,
    /// with visibility computed against the active workspace name.
    #[cfg(feature = "workspaces+hyprland")]
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

    /// Gets the active workspace from the server.
    #[cfg(feature = "workspaces+hyprland")]
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
        match request(&command) {
            Ok(reply) if !reply.starts_with("ok") => {
                error!("dispatch '{command}' rejected: {reply}");
            }
            Err(err) => error!("dispatch '{command}' failed: {err:#}"),
            _ => {}
        }
    }

    fn subscribe(&self) -> Receiver<WorkspaceUpdate> {
        let rx = self.workspace.tx.subscribe();

        let active = Self::get_active_workspace().ok().map(|w| w.name);
        match Self::get_workspaces(active.as_deref()) {
            Ok(workspaces) => self
                .workspace
                .tx
                .send_expect(WorkspaceUpdate::Init(workspaces)),
            Err(err) => error!("Failed to get workspaces: {err:#}"),
        }

        rx
    }
}

#[cfg(feature = "keyboard+hyprland")]
impl KeyboardLayoutClient for Client {
    fn set_next_active(&self) {
        if let Some(keyboard) = main_keyboard()
            && let Err(err) = request(&format!("switchxkblayout {} next", keyboard.name))
        {
            error!("Failed to switch keyboard layout: {err:#}");
        }
    }

    fn subscribe(&self) -> Receiver<KeyboardLayoutUpdate> {
        let rx = self.keyboard_layout.tx.subscribe();

        if let Some(keyboard) = main_keyboard() {
            self.keyboard_layout
                .tx
                .send_expect(KeyboardLayoutUpdate(keyboard.active_keymap));
        }

        rx
    }
}

#[cfg(feature = "bindmode+hyprland")]
impl BindModeClient for Client {
    fn subscribe(&self) -> super::Result<Receiver<BindModeUpdate>> {
        Ok(self.bindmode.tx.subscribe())
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
    match request_json::<HDevices>("j/devices") {
        Ok(devices) => {
            let keyboard = devices.keyboards.into_iter().find(|k| k.main);
            if keyboard.is_none() {
                error!("Failed to get keyboard device from hyprland");
            }
            keyboard
        }
        Err(err) => {
            error!("Failed to get devices: {err:#}");
            None
        }
    }
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
