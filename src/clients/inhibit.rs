use crate::register_client;
use gtk::ApplicationInhibitFlags;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, trace};

fn get_app() -> gtk::Application {
    gtk::gio::Application::default()
        .and_downcast()
        .expect("GTK application not initialized")
}

/// Uninhibits on drop.
#[derive(Debug)]
struct InhibitCookie(u32);

impl Drop for InhibitCookie {
    fn drop(&mut self) {
        get_app().uninhibit(self.0);
    }
}

fn gtk_inhibit() -> Option<InhibitCookie> {
    let id = get_app().inhibit(
        None::<&gtk::Window>,
        ApplicationInhibitFlags::IDLE,
        Some("Ironbar inhibit"),
    );
    if id == 0 {
        error!("GTK inhibit failed - platform may not support it");
        None
    } else {
        trace!("created inhibit cookie: {id}");
        Some(InhibitCookie(id))
    }
}

/// The live inhibit: the held GTK cookie and its countdown, or `None` when
/// inactive. Bundling them makes "active without a cookie" unrepresentable.
#[derive(Default, Debug)]
struct Inhibitor {
    state: Option<(InhibitCookie, Duration)>,
}

impl Inhibitor {
    /// Remaining inhibit time, or `None` when inactive.
    fn remaining(&self) -> Option<Duration> {
        self.state.as_ref().map(|(_, duration)| *duration)
    }

    /// True while a finite countdown is running.
    fn is_counting_down(&self) -> bool {
        self.remaining()
            .is_some_and(|duration| duration != Duration::MAX)
    }

    /// `Some` starts/retargets, `None` stops; a failed GTK acquire stays inactive.
    fn set(&mut self, target: Option<Duration>) {
        match target {
            None => self.state = None,
            Some(duration) => {
                if let Some((_, current)) = &mut self.state {
                    *current = duration;
                } else {
                    self.state = gtk_inhibit().map(|cookie| (cookie, duration));
                }
            }
        }
    }

    /// Count down a second, dropping the cookie when the countdown hits zero. Only
    /// called while `is_counting_down`, so the duration is always finite here.
    fn tick(&mut self) {
        let expired = match &mut self.state {
            Some((_, duration)) => {
                *duration = duration.saturating_sub(Duration::from_secs(1));
                *duration == Duration::ZERO
            }
            None => false,
        };
        if expired {
            self.state = None;
        }
    }
}

/// Process-global inhibit: owns the GTK cookie and live countdown as shared
/// main-thread state. Every bar reads and drives it directly - no request channel.
#[derive(Debug)]
pub struct Client {
    state: Rc<RefCell<Inhibitor>>,
    state_tx: watch::Sender<Option<Duration>>,
}

impl Client {
    /// Creates the client and spawns its countdown task. Must be obtained on the
    /// GTK main thread - only the inhibit module uses it, so it always is.
    pub(crate) fn new() -> Self {
        let state = Rc::<RefCell<Inhibitor>>::default();
        let (state_tx, _) = watch::channel(None::<Duration>);

        glib::spawn_future_local({
            let (state, state_tx) = (state.clone(), state_tx.clone());
            async move {
                loop {
                    glib::timeout_future_seconds(1).await;
                    let mut inhibitor = state.borrow_mut();
                    if !inhibitor.is_counting_down() {
                        continue;
                    }
                    inhibitor.tick();
                    let remaining = inhibitor.remaining();
                    drop(inhibitor);
                    state_tx.send_replace(remaining);
                }
            }
        });

        Self { state, state_tx }
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<Duration>> {
        self.state_tx.subscribe()
    }

    /// Currently remaining inhibit time, or `None` when inactive.
    pub fn remaining(&self) -> Option<Duration> {
        self.state.borrow().remaining()
    }

    /// `Some` starts/retargets the inhibit, `None` stops it. `Duration::MAX` = infinite.
    pub fn set_duration(&self, duration: Option<Duration>) {
        let mut inhibitor = self.state.borrow_mut();
        inhibitor.set(duration);
        let remaining = inhibitor.remaining();
        drop(inhibitor);
        self.state_tx.send_replace(remaining);
    }
}

register_client!(Client, inhibit);
