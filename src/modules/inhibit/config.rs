use crate::config::{CommonConfig, LayoutConfig};
use chrono::Timelike;
use serde::{Deserialize, Deserializer};
use std::time::Duration;

/// Command to control inhibit state.
///
/// **Valid options**: `toggle`, `cycle`
#[derive(Debug, Clone)]
pub enum InhibitCommand {
    /// Toggle inhibit on/off
    Toggle,
    /// Cycle to next duration (or turn off if active)
    Cycle,
}

// Config representation without surface (for deserialization)
#[derive(Debug, Deserialize, Clone, Copy)]
#[cfg_attr(feature = "extras", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum InhibitAction {
    Toggle,
    Cycle,
}

impl From<InhibitAction> for InhibitCommand {
    fn from(action: InhibitAction) -> Self {
        match action {
            InhibitAction::Toggle => Self::Toggle,
            InhibitAction::Cycle => Self::Cycle,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[cfg_attr(feature = "extras", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct InhibitModule {
    /// List of durations to cycle through.
    /// Use `0` for infinite inhibit.
    /// Format: `HH:MM:SS` (e.g., `01:30:00` for 1 hour 30 minutes)
    ///
    /// **Default**: `["00:30:00", "01:00:00", "01:30:00", "02:00:00", "0"]`
    #[serde(deserialize_with = "deserialize_durations")]
    #[cfg_attr(feature = "extras", schemars(with = "Option<Vec<String>>"))]
    pub(super) durations: Vec<Duration>,

    /// The default duration to use when starting inhibit.
    /// Must be one of the values in `durations`.
    /// Format: `HH:MM:SS` (e.g., `02:00:00` for 2 hours)
    ///
    /// **Default**: `"02:00:00"`
    #[serde(deserialize_with = "deserialize_default_duration")]
    #[cfg_attr(feature = "extras", schemars(with = "Option<String>"))]
    pub(super) default_duration: Duration,

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
        let default_durations = vec![
            Duration::from_secs(30 * 60),  // 00:30:00
            Duration::from_secs(60 * 60),  // 01:00:00
            Duration::from_secs(90 * 60),  // 01:30:00
            Duration::from_secs(120 * 60), // 02:00:00
            Duration::MAX,                 // 0 (infinite)
        ];
        let default_duration = Duration::from_secs(120 * 60); // 02:00:00

        Self {
            durations: default_durations,
            default_duration,
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

fn parse_duration(s: &str) -> color_eyre::Result<Duration> {
    if s == "0" {
        return Ok(Duration::MAX);
    }

    let time = chrono::NaiveTime::parse_from_str(s, "%H:%M:%S").map_err(|_| {
        color_eyre::eyre::eyre!("Invalid duration format. Use HH:MM:SS (e.g., 01:30:00)")
    })?;

    let secs = time.hour() as u64 * 3600 + time.minute() as u64 * 60 + time.second() as u64;
    Ok(Duration::from_secs(secs))
}

fn parse_durations(strings: Vec<String>) -> color_eyre::Result<Vec<Duration>> {
    let strings_to_parse: Vec<&str> = if strings.is_empty() {
        vec!["00:30:00", "01:00:00", "01:30:00", "02:00:00", "0"]
    } else {
        strings.iter().map(|s| s.as_str()).collect()
    };

    let mut durations = Vec::new();
    for s in strings_to_parse {
        durations.push(parse_duration(s)?);
    }

    Ok(durations)
}

fn deserialize_durations<'de, D>(deserializer: D) -> Result<Vec<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    parse_durations(Vec::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

fn deserialize_default_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = String::deserialize(deserializer)?;
    parse_duration(&s).map_err(serde::de::Error::custom)
}
