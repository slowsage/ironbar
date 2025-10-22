use chrono::Utc;
use color_eyre::Result;
use gtk::prelude::*;
use gtk::{Button, Label};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::channels::{AsyncSenderExt, BroadcastReceiverExt};
use crate::clients::inhibit;
use crate::gtk_helpers::{IronbarGtkExt, MouseButton};
use crate::modules::{Module, ModuleInfo, ModuleParts, ModuleUpdateEvent, WidgetContext};
use crate::{module_impl, spawn};
use wayland_client::protocol::wl_surface::WlSurface;

mod config;

use config::InhibitAction;
pub use config::{InhibitCommand, InhibitModule};

/// Extract WlSurface from a GTK widget.
fn extract_wl_surface(widget: &impl gtk::prelude::IsA<gtk::Widget>) -> Option<WlSurface> {
    use gdk4_wayland::prelude::WaylandSurfaceExtManual;

    let window = widget.root()?.downcast::<gtk::Window>().ok()?;
    let gdk_surface = window.surface()?;
    let wayland_surface = gdk_surface.downcast::<gdk4_wayland::WaylandSurface>().ok()?;
    wayland_surface.wl_surface()
}

/// Format a duration for display.
fn format_duration(duration: Duration) -> String {
    if duration == Duration::MAX {
        return "∞".to_string();
    }

    let secs = duration.as_secs();
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);

    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else if m > 0 {
        format!("{:02}:{:02}", m, s)
    } else {
        format!("{}s", s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Inactive { selected_duration: Duration },
    Active { remaining: Duration },
}

fn get_state(client: &Arc<inhibit::Client>, selected_duration: Duration) -> State {
    match client.inhibit_expiry() {
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
    client: &Arc<inhibit::Client>,
    durations: &[Duration],
    idx: &mut usize,
    tx: &impl AsyncSenderExt<ModuleUpdateEvent<State>>,
) -> Result<State> {
    let current_state = get_state(client, durations[*idx]);

    match (cmd, current_state) {
        (InhibitCommand::Toggle, State::Active { .. }) => {
            client.stop_inhibit().await?;
        }
        (InhibitCommand::Toggle, _) => {
            client.start_inhibit(durations[*idx]).await?;
        }
        (InhibitCommand::Cycle, current) => {
            *idx = (*idx + 1) % durations.len();
            if matches!(current, State::Active { .. }) {
                client.start_inhibit(durations[*idx]).await?;
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
        let duration_list = self.durations.clone();
        let default_duration = self.default_duration;

        // Get inhibit client with backend
        let backend = self.backend;
        let client = ctx.ironbar.clients.borrow_mut().inhibit(backend)?;

        spawn(async move {
            let mut idx = duration_list
                .iter()
                .position(|d| *d == default_duration)
                .unwrap_or(0);
            let mut state = get_state(&client, duration_list[idx]);
            tx.send_update(state.clone()).await;
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.tick().await;
            loop {
                tokio::select! {
                    Some(cmd) = rx.recv() => {
                        if let Ok(new_state) = handle_command(cmd, &client, &duration_list, &mut idx, &tx).await {
                            state = new_state;
                        }
                    }
                    _ = interval.tick() => {
                        let new_state = get_state(&client, duration_list[idx]);
                        if matches!(new_state, State::Inactive { .. }) && !matches!(state, State::Inactive { .. }) {
                            client.stop_inhibit().await.ok();
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
        _info: &ModuleInfo,
    ) -> Result<ModuleParts<Button>> {
        let button = Button::new();
        button.add_css_class("inhibit");
        let label = Label::builder()
            .use_markup(true)
            .justify(self.layout.justify.into())
            .build();
        button.set_child(Some(&label));
        let tx = ctx.controller_tx.clone();

        // Initialize surface when button is realized (only for Wayland backend)
        if matches!(self.backend, inhibit::BackendType::Wayland) {
            use crate::clients::ProvidesClient;
            let wayland_client: Arc<crate::clients::gtk_wayland::Client> = ctx.provide();
            button.connect_realize(move |btn| {
                tracing::debug!("Button realized, initializing idle inhibit surface");
                if let Some(wl_surface) = extract_wl_surface(btn) {
                    tracing::debug!("Got WlSurface, initializing idle inhibit manager");
                    wayland_client.idle_inhibit().init_surface(wl_surface);
                } else {
                    tracing::error!("Failed to extract Wayland surface");
                }
            });
        }

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

            button.connect_pressed(btn, move || {
                tracing::debug!("Inhibit button clicked: {:?}", btn);

                // Convert InhibitAction to InhibitCommand (no surface needed)
                let command = match action {
                    InhibitAction::Toggle => InhibitCommand::Toggle,
                    InhibitAction::Cycle => InhibitCommand::Cycle,
                };

                tracing::debug!(
                    "Sending inhibit command: {:?}",
                    std::mem::discriminant(&command)
                );
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
            let duration_str = format!("{:>7}", format_duration(duration));
            let text = format.replace("{duration}", &duration_str);
            label.set_label(&text);
        });
        Ok(ModuleParts::new(button, None))
    }
}
