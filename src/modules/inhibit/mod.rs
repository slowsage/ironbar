use chrono::Utc;
use color_eyre::Result;
use gtk::prelude::*;
use gtk::{Button, Label};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::channels::{AsyncSenderExt, BroadcastReceiverExt};
use crate::clients::inhibit::InhibitClient;
use crate::gtk_helpers::{IronbarGtkExt, MouseButton};
use crate::modules::{Module, ModuleInfo, ModuleParts, ModuleUpdateEvent, WidgetContext};
use crate::{module_impl, spawn};

mod config;

use config::InhibitAction;
pub use config::{InhibitCommand, InhibitModule};

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Inactive { selected_duration: Duration },
    Active { remaining: Duration },
}

fn get_state(client: &InhibitClient, selected_duration: Duration) -> State {
    match client.expiry() {
        None => State::Inactive { selected_duration },
        Some(expiry_time) if expiry_time == chrono::DateTime::<Utc>::MAX_UTC => State::Active {
            remaining: Duration::MAX,
        },
        Some(expiry_time) => match (expiry_time - Utc::now()).to_std().map(|d| d.as_secs()) {
            Ok(secs) if secs > 0 => State::Active {
                remaining: Duration::from_secs(secs),
            },
            _ => State::Inactive { selected_duration },
        },
    }
}

async fn handle_command(
    cmd: InhibitCommand,
    client: &mut InhibitClient,
    durations: &[Duration],
    idx: &mut usize,
    tx: &impl AsyncSenderExt<ModuleUpdateEvent<State>>,
) -> Result<State> {
    let current_state = get_state(client, durations[*idx]);

    match (cmd, current_state) {
        (InhibitCommand::Toggle(_), State::Active { .. }) => {
            client.stop().await.ok();
        }
        (InhibitCommand::Toggle(surface), _) => {
            if let Some(surf) = surface {
                client.start(durations[*idx], surf).await.ok();
            }
        }
        (InhibitCommand::Cycle(surface), current) => {
            *idx = (*idx + 1) % durations.len();
            if matches!(current, State::Active { .. }) {
                if let Some(surf) = surface {
                    client.start(durations[*idx], surf).await?;
                }
            }
        }
    }
    let new_state = get_state(client, durations[*idx]);
    tx.send_update(new_state.clone()).await;
    Ok(new_state)
}

impl Module<Button> for InhibitModule {
    type SendMessage = State;
    type ReceiveMessage = InhibitCommand;

    module_impl!("inhibit");

    fn spawn_controller(
        &self,
        _info: &ModuleInfo,
        ctx: &WidgetContext<Self::SendMessage, Self::ReceiveMessage>,
        mut rx: mpsc::Receiver<Self::ReceiveMessage>,
    ) -> Result<()> {
        let tx = ctx.tx.clone();
        let (duration_list, default_index) = self.durations.clone();

        let backend_type = self.backend;

        // Create inhibit client
        let mut client = ctx.ironbar.clients.borrow_mut().inhibit(backend_type)?;

        spawn(async move {
            let mut idx = default_index;
            let mut state = get_state(&client, duration_list[idx]);
            tx.send_update(state.clone()).await;
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.tick().await;
            loop {
                tokio::select! {
                    Some(cmd) = rx.recv() => {
                        if let Ok(new_state) = handle_command(cmd, &mut client, &duration_list, &mut idx, &tx).await {
                            state = new_state;
                        }
                    }
                    _ = interval.tick() => {
                        let new_state = get_state(&client, duration_list[idx]);
                        if matches!(new_state, State::Inactive { .. }) && !matches!(state, State::Inactive { .. }) {
                            client.stop().await.ok();
                        }
                        if state != new_state {
                            state = new_state.clone();
                            tx.send_update(new_state).await;
                        }
                    }
                }
            }
        });
        Ok(())
    }

    fn into_widget(
        self,
        ctx: WidgetContext<Self::SendMessage, Self::ReceiveMessage>,
        info: &ModuleInfo,
    ) -> Result<ModuleParts<Button>> {
        let button = Button::new();
        button.add_css_class("inhibit");
        let label = Label::builder()
            .use_markup(true)
            .justify(self.layout.justify.into())
            .build();
        button.set_child(Some(&label));
        let tx = ctx.controller_tx.clone();
        let backend_type = self.backend;
        let app = info.app.clone();

        // Bind mouse buttons to actions
        [
            (MouseButton::Primary, self.on_click_left),
            (MouseButton::Secondary, self.on_click_right),
            (MouseButton::Middle, self.on_click_middle),
        ]
        .into_iter()
        .filter_map(|(btn, cmd)| cmd.map(|c| (btn, c)))
        .for_each(|(btn, action)| {
            let tx = tx.clone();
            let app_btn = app.clone();
            let backend = backend_type;

            button.connect_pressed(btn, move || {
                tracing::debug!("Inhibit button clicked: {:?}", btn);

                // Get surface ID on GTK main thread
                let surface_id = if backend == crate::clients::inhibit::BackendType::Wayland {
                    match crate::clients::wayland::get_gtk_wayland_surface_id(&app_btn) {
                        Ok(id) => {
                            tracing::debug!("Got GTK Wayland surface ID: {:?}", id);
                            Some(id)
                        }
                        Err(e) => {
                            tracing::error!("Failed to get GTK Wayland surface ID: {}", e);
                            None
                        }
                    }
                } else {
                    tracing::debug!("Using systemd backend, no surface needed");
                    None
                };

                // Convert InhibitAction to InhibitCommand with surface ID
                let command = match action {
                    InhibitAction::Toggle => InhibitCommand::Toggle(surface_id),
                    InhibitAction::Cycle => InhibitCommand::Cycle(surface_id),
                };

                tracing::debug!("Sending inhibit command: {:?}", std::mem::discriminant(&command));
                let tx = tx.clone();
                spawn(async move {
                    if let Err(e) = tx.send(command).await {
                        tracing::error!("Failed to send inhibit command: {}", e);
                    } else {
                        tracing::debug!("Inhibit command sent successfully");
                    }
                });
            });
        });
        let rx = ctx.subscribe();
        let format_on = self.format_on;
        let format_off = self.format_off;
        rx.recv_glib(&label, move |label, state| {
            let (format, duration) = match state {
                State::Active { remaining } => (&format_on, remaining),
                State::Inactive { selected_duration } => (&format_off, selected_duration),
            };
            let duration_str = if duration == Duration::MAX {
                format!("{:>7}", "∞")
            } else {
                let secs = duration.as_secs();
                let h = secs / 3600;
                let m = (secs % 3600) / 60;
                let s = secs % 60;
                if h > 0 {
                    format!("{:>7}", format!("{:02}:{:02}:{:02}", h, m, s))
                } else if m > 0 {
                    format!("{:>7}", format!("{:02}:{:02}", m, s))
                } else {
                    format!("{:>7}", format!("{}s", s))
                }
            };
            let text = format.replace("{duration}", &duration_str);
            label.set_label(&text);
        });
        Ok(ModuleParts::new(button, None))
    }
}
