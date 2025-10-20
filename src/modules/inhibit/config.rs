use crate::clients::inhibit::BackendType;
use crate::config::{CommonConfig, LayoutConfig};
use serde::{Deserialize, Deserializer};
use std::time::Duration;

/// Command to control inhibit state.
///
/// **Valid options**: `toggle`, `cycle`
#[derive(Debug, Clone)]
pub enum InhibitCommand {
    /// Toggle inhibit on/off
    Toggle(Option<wayland_client::backend::ObjectId>),
    /// Cycle to next duration (or turn off if active)
    Cycle(Option<wayland_client::backend::ObjectId>),
}

// Config representation without surface (for deserialization)
#[derive(Debug, Deserialize, Clone, Copy)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum InhibitAction {
    Toggle,
    Cycle,
}

#[derive(Debug, Deserialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct InhibitModule {
    /// Backend to use for inhibiting idle/sleep.
    ///
    /// **Valid options**: `systemd`, `wayland`
    /// <br>
    /// **Default**: `wayland`
    #[serde(default)]
    pub(super) backend: BackendType,

    /// List of durations to cycle through.
    /// Prefix a duration with `*` to mark it as the default.
    /// Use `0` for infinite inhibit.
    /// Format: `HH:MM:SS` (e.g., `01:30:00` for 1 hour 30 minutes)
    ///
    /// **Default**: `["00:30:00", "01:00:00", "01:30:00", "*02:00:00", "0"]`
    #[serde(deserialize_with = "deserialize_durations")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<Vec<String>>"))]
    pub(super) durations: (Vec<Duration>, usize),

    /// Command to execute on left click.
    ///
    /// **Valid options**: `toggle`, `cycle`
    /// <br>
    /// **Default**: `toggle`
    pub(super) on_click_left: Option<InhibitAction>,

    /// Command to execute on right click.
    ///
    /// **Valid options**: `toggle`, `cycle`
    /// <br>
    /// **Default**: `cycle`
    pub(super) on_click_right: Option<InhibitAction>,

    /// Command to execute on middle click.
    ///
    /// **Valid options**: `toggle`, `cycle`
    /// <br>
    /// **Default**: `null`
    pub(super) on_click_middle: Option<InhibitAction>,

    /// Format string when inhibit is active.
    /// `{duration}` token shows remaining/selected time.
    ///
    /// **Default**: `"☕ {duration}"`
    pub(super) format_on: String,

    /// Format string when inhibit is inactive.
    /// `{duration}` token shows selected duration.
    ///
    /// **Default**: `"💤 {duration}"`
    pub(super) format_off: String,

    /// See [layout options](module-level-options#layout-options).
    #[serde(flatten)]
    pub(super) layout: LayoutConfig,

    /// See [common options](module-level-options#common-options).
    #[serde(flatten)]
    pub common: Option<CommonConfig>,
}

impl Default for InhibitModule {
    fn default() -> Self {
        Self {
            backend: BackendType::Wayland,
            durations: parse_durations_with_default(Vec::new()).unwrap(),
            on_click_left: Some(InhibitAction::Toggle),
            on_click_right: Some(InhibitAction::Cycle),
            on_click_middle: None,
            format_on: "☕ {duration}".to_string(),
            format_off: "💤 {duration}".to_string(),
            layout: LayoutConfig::default(),
            common: None,
        }
    }
}

fn parse_duration(s: &str) -> color_eyre::Result<(Duration, bool)> {
    use chrono::{NaiveTime, Timelike};

    // "*" prefix marks which duration is the default selection
    let is_default = s.trim().starts_with('*');
    let s = s.trim().trim_start_matches('*').trim();
    let duration = if s == "0" {
        Duration::MAX
    } else {
        let time = NaiveTime::parse_from_str(s, "%H:%M:%S")
            .map_err(|_| color_eyre::eyre::eyre!("Invalid duration format. Use HH:MM:SS (e.g., 01:30:00)"))?;
        let secs = time.hour() as u64 * 3600 + time.minute() as u64 * 60 + time.second() as u64;
        Duration::from_secs(secs)
    };
    Ok((duration, is_default))
}

fn parse_durations_with_default(
    strings: Vec<String>,
) -> color_eyre::Result<(Vec<Duration>, usize)> {
    let strings_to_parse: Vec<&str> = if strings.is_empty() {
        vec!["00:30:00", "01:00:00", "01:30:00", "*02:00:00", "0"]
    } else {
        strings.iter().map(|s| s.as_str()).collect()
    };

    let (mut durations, mut default_idx) = (Vec::new(), strings_to_parse.len() - 1);

    for (i, s) in strings_to_parse.iter().enumerate() {
        let (duration, is_default) = parse_duration(s)?;
        if is_default {
            default_idx = i;
        }
        durations.push(duration);
    }

    Ok((durations, default_idx))
}

fn deserialize_durations<'de, D>(deserializer: D) -> Result<(Vec<Duration>, usize), D::Error>
where
    D: Deserializer<'de>,
{
    parse_durations_with_default(Vec::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}
