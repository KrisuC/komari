use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};

use super::impl_identifiable;
use crate::{KeyBinding, KeyBindingConfiguration};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(skip_serializing, default)]
    pub id: Option<i64>,
    pub capture_mode: CaptureMode,
    #[serde(default = "enable_solving_default")]
    pub enable_rune_solving: bool,
    #[serde(default = "enable_solving_default")]
    pub enable_transparent_shape_solving: bool,
    #[serde(default = "enable_solving_default")]
    pub enable_violetta_solving: bool,
    pub enable_panic_mode: bool,
    pub stop_on_fail_or_change_map: bool,
    #[serde(default = "stop_on_player_die_default")]
    pub stop_on_player_die: bool,
    #[serde(default)]
    pub run_timer: bool,
    #[serde(default = "run_timer_millis_default")]
    pub run_timer_millis: u64,
    pub input_method: InputMethod,
    pub input_method_rpc_server_url: String,
    pub notifications: Notifications,
    #[serde(default = "toggle_actions_key_default")]
    pub toggle_actions_key: KeyBindingConfiguration,
    #[serde(default = "platform_start_key_default")]
    pub platform_start_key: KeyBindingConfiguration,
    #[serde(default = "platform_end_key_default")]
    pub platform_end_key: KeyBindingConfiguration,
    #[serde(default = "platform_add_key_default")]
    pub platform_add_key: KeyBindingConfiguration,
    /// The last selected character id to auto-select on startup.
    #[serde(default)]
    pub selected_character_id: Option<i64>,
    /// The last selected map id to auto-select on startup.
    #[serde(default)]
    pub selected_map_id: Option<i64>,
    /// The last selected actions preset name to auto-select on startup.
    #[serde(default)]
    pub selected_map_preset: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            id: None,
            capture_mode: CaptureMode::default(),
            enable_rune_solving: enable_solving_default(),
            enable_transparent_shape_solving: enable_solving_default(),
            enable_violetta_solving: enable_solving_default(),
            enable_panic_mode: false,
            input_method: InputMethod::default(),
            input_method_rpc_server_url: String::default(),
            stop_on_fail_or_change_map: false,
            stop_on_player_die: stop_on_player_die_default(),
            run_timer: false,
            run_timer_millis: run_timer_millis_default(),
            notifications: Notifications::default(),
            toggle_actions_key: toggle_actions_key_default(),
            platform_start_key: platform_start_key_default(),
            platform_end_key: platform_end_key_default(),
            platform_add_key: platform_add_key_default(),
            selected_character_id: None,
            selected_map_id: None,
            selected_map_preset: None,
        }
    }
}

impl_identifiable!(Settings);

fn stop_on_player_die_default() -> bool {
    true
}

fn run_timer_millis_default() -> u64 {
    14400000 // 4 hours
}

fn enable_solving_default() -> bool {
    true
}

fn toggle_actions_key_default() -> KeyBindingConfiguration {
    KeyBindingConfiguration {
        key: KeyBinding::Comma,
        enabled: false,
    }
}

fn platform_start_key_default() -> KeyBindingConfiguration {
    KeyBindingConfiguration {
        key: KeyBinding::J,
        enabled: false,
    }
}

fn platform_end_key_default() -> KeyBindingConfiguration {
    KeyBindingConfiguration {
        key: KeyBinding::K,
        enabled: false,
    }
}

fn platform_add_key_default() -> KeyBindingConfiguration {
    KeyBindingConfiguration {
        key: KeyBinding::L,
        enabled: false,
    }
}

#[derive(
    Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum InputMethod {
    #[default]
    Default,
    Rpc,
}

#[derive(
    Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum CaptureMode {
    BitBlt,
    #[strum(to_string = "Windows 10 (1903 and up)")] // Thanks OBS
    #[default]
    WindowsGraphicsCapture,
    BitBltArea,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Notifications {
    pub discord_user_id: String,
    #[serde(default, alias = "discord_webhook_url")]
    pub webhook_url: String,
    #[serde(default)]
    pub webhook_provider: WebhookProvider,
    pub notify_on_fail_or_change_map: bool,
    pub notify_on_rune_appear: bool,
    pub notify_on_elite_boss_appear: bool,
    pub notify_on_player_die: bool,
    pub notify_on_player_guildie_appear: bool,
    pub notify_on_player_stranger_appear: bool,
    pub notify_on_player_friend_appear: bool,
    #[serde(default)]
    pub notify_on_lie_detector_appear: bool,
    #[serde(default)]
    pub notify_on_run_timer_end: bool,
}

#[derive(
    Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum WebhookProvider {
    #[default]
    Discord,
}
