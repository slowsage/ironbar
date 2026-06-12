use color_eyre::Result;
use gtk::prelude::*;
use gtk::{Button, Label, glib};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

use crate::clients::inhibit;
use crate::gtk_helpers::{IronbarGtkExt, IronbarLabelExt, MouseButton};
use crate::module_impl;
use crate::modules::{Module, ModuleInfo, ModuleParts, WidgetContext};

mod config;

use config::InhibitCommand;
pub use config::InhibitModule;

const INFINITE_DURATION_LABEL: &str = "\u{edfe}";

fn format_duration(d: Duration) -> String {
    if d == Duration::MAX {
        return INFINITE_DURATION_LABEL.to_string();
    }
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, s % 3600 / 60, s % 60);
    match (h, m) {
        (h, m) if h > 0 => format!("{h:02}:{m:02}:{s:02}"),
        (_, m) => format!("{m:02}:{s:02}"),
    }
}

fn format_state(fmt: &str, duration: Duration) -> String {
    fmt.replace("{duration}", &format_duration(duration))
}

impl Module<Button> for InhibitModule {
    // The shared inhibit state lives in the client; the widget reads and drives it
    // directly on the GTK thread, so there is no controller and no messages.
    type SendMessage = ();
    type ReceiveMessage = ();

    module_impl!("inhibit");

    fn spawn_controller(
        &self,
        _info: &ModuleInfo,
        _ctx: &WidgetContext<(), ()>,
        _rx: Receiver<()>,
    ) -> Result<()> {
        Ok(())
    }

    fn into_widget(
        self,
        ctx: WidgetContext<(), ()>,
        _info: &ModuleInfo,
    ) -> Result<ModuleParts<Button>> {
        let button = Button::new();
        button.add_css_class("inhibit");
        let label = Label::builder()
            .use_markup(true)
            .justify(self.layout.justify.into())
            .build();
        button.set_child(Some(&label));

        let client = ctx.client::<inhibit::Client>();
        let durations = Rc::new(self.duration_spec.durations);
        let idx = Rc::new(Cell::new(
            durations
                .iter()
                .position(|d| *d == self.duration_spec.default_duration)
                .unwrap_or(0),
        ));

        // Render from the shared countdown when active, this bar's own preset when idle.
        let render: Rc<dyn Fn()> = {
            let (label, client, durations, idx) =
                (label, client.clone(), durations.clone(), idx.clone());
            let (fmt_on, fmt_off) = (self.format_on, self.format_off);
            Rc::new(move || {
                let (fmt, duration) = match client.remaining() {
                    Some(remaining) => (&fmt_on, remaining),
                    None => (&fmt_off, durations[idx.get()]),
                };
                label.set_label_escaped(&format_state(fmt, duration));
            })
        };

        // Re-render on every shared-state change (countdown ticks, other bars).
        glib::spawn_future_local({
            let (render, mut state_rx) = (render.clone(), client.subscribe());
            async move {
                render();
                while state_rx.changed().await.is_ok() {
                    render();
                }
            }
        });

        // Clicks mutate this bar's preset / the shared inhibit, then re-render.
        [
            (MouseButton::Primary, self.on_click_left),
            (MouseButton::Secondary, self.on_click_right),
            (MouseButton::Middle, self.on_click_middle),
        ]
        .into_iter()
        .filter_map(|(btn, cmd)| cmd.map(|c| (btn, c)))
        .for_each(|(btn, cmd)| {
            let (client, durations, idx, render) =
                (client.clone(), durations.clone(), idx.clone(), render.clone());
            button.connect_pressed(btn, move || {
                let active = client.remaining().is_some();
                match cmd {
                    InhibitCommand::Toggle if active => client.set_duration(None),
                    InhibitCommand::Toggle => client.set_duration(Some(durations[idx.get()])),
                    InhibitCommand::Cycle => {
                        idx.set((idx.get() + 1) % durations.len());
                        if active {
                            client.set_duration(Some(durations[idx.get()]));
                        }
                    }
                }
                render();
            });
        });

        Ok(ModuleParts::new(button, None))
    }
}
