use crate::ironvar::WritableNamespace;
use crate::modules::inhibit::config::InhibitCommand;
use crate::modules::inhibit::{State, format_duration};
use crate::{Ironbar, spawn};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, warn};

pub fn start(command_tx: mpsc::Sender<InhibitCommand>, mut state_rx: broadcast::Receiver<State>) {
    spawn(async move {
        debug!("Inhibit IPC controller started");

        let vm = Ironbar::variable_manager();
        let mut cmd_sub = vm.subscribe("inhibit_cmd".into());

        loop {
            tokio::select! {
                // Handle incoming commands from ironvars
                Ok(Some(msg)) = cmd_sub.recv() => {
                    let cmd = match msg.as_str() {
                        "toggle" => Some(InhibitCommand::Toggle),
                        "cycle" => Some(InhibitCommand::Cycle),
                        "" => None, // Ignore reset
                        _ => {
                            warn!("Unknown inhibit command: {}", msg);
                            None
                        }
                    };

                    if let Some(cmd) = cmd {
                        if let Err(e) = command_tx.send(cmd).await {
                            error!("Failed to send inhibit command: {}", e);
                            break;
                        }

                        // Reset variable to allow re-triggering
                        if let Err(e) = vm.set("inhibit_cmd", String::new()) {
                            error!("Failed to reset inhibit_cmd: {}", e);
                        }
                    }
                }

                // Publish state changes to ironvar
                Ok(state) = state_rx.recv() => {
                    let status = if state.active { "active" } else { "inactive" };
                    let info = format!(
                        r#"{{"status":"{}","remaining":"{}"}}"#,
                        status,
                        format_duration(state.duration)
                    );

                    if let Err(e) = vm.set("inhibit_info", info) {
                        error!("Failed to set inhibit_info: {}", e);
                    }
                }
            }
        }
    });
}
