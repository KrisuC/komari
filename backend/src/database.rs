use std::{
    collections::{HashMap, HashSet},
    env,
    sync::{LazyLock, Mutex},
};

use anyhow::{Result, bail};
use opencv::core::Rect;
use rusqlite::{Connection, Params, Statement, types::Null};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use serde_json::Value;
use strum::{Display, EnumIter, EnumString};
use tokio::sync::broadcast::{Receiver, Sender, channel};

use crate::bridge::KeyKind;
use crate::pathing;

const MAPS: &str = "maps";
const NAVIGATION_PATHS: &str = "navigation_paths";
const CHARACTERS: &str = "characters";
const SETTINGS: &str = "settings";
const SEEDS: &str = "seeds";

static CONNECTION: LazyLock<Mutex<Connection>> = LazyLock::new(|| {
    let path = env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("local.db")
        .to_path_buf();
    let conn = Connection::open(path.to_str().unwrap()).expect("failed to open local.db");
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS maps (
            id INTEGER PRIMARY KEY,
            data TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS navigation_paths (
            id INTEGER PRIMARY KEY,
            data TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS characters (
            id INTEGER PRIMARY KEY,
            data TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY,
            data TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS seeds (
            id INTEGER PRIMARY KEY,
            data TEXT NOT NULL
        );
        "#,
    )
    .unwrap();
    Mutex::new(conn)
});
static EVENT: LazyLock<Sender<DatabaseEvent>> = LazyLock::new(|| channel(10).0);

#[derive(Debug, Clone)]
pub enum DatabaseEvent {
    MinimapUpdated(Minimap),
    MinimapDeleted(i64),
    NavigationPathsUpdated,
    NavigationPathsDeleted,
    SettingsUpdated(Settings),
    CharacterUpdated(Character),
    CharacterDeleted(i64),
}

trait Identifiable {
    fn id(&self) -> Option<i64>;

    fn set_id(&mut self, id: i64);
}

macro_rules! impl_identifiable {
    ($type:ty) => {
        impl Identifiable for $type {
            fn id(&self) -> Option<i64> {
                self.id
            }

            fn set_id(&mut self, id: i64) {
                self.id = Some(id);
            }
        }
    };
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seeds {
    pub id: Option<i64>,
    pub seed: [u8; 32],
}

impl Default for Seeds {
    fn default() -> Self {
        Self {
            id: None,
            seed: rand::random(),
        }
    }
}

impl_identifiable!(Seeds);

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
pub enum SwappableFamiliars {
    #[default]
    All,
    Last,
    SecondAndLast,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default, Hash, Serialize, Deserialize)]
pub enum FamiliarRarity {
    #[default]
    Rare,
    Epic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Familiars {
    pub enable_familiars_swapping: bool,
    #[serde(default = "familiars_swap_check_millis")]
    pub swap_check_millis: u64,
    pub swappable_familiars: SwappableFamiliars,
    pub swappable_rarities: HashSet<FamiliarRarity>,
}

impl Default for Familiars {
    fn default() -> Self {
        Self {
            enable_familiars_swapping: false,
            swap_check_millis: familiars_swap_check_millis(),
            swappable_familiars: SwappableFamiliars::default(),
            swappable_rarities: HashSet::default(),
        }
    }
}

fn familiars_swap_check_millis() -> u64 {
    300000
}

#[derive(
    Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum EliteBossBehavior {
    #[default]
    None,
    CycleChannel,
    UseKey,
}

#[derive(
    Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum CycleRunStopMode {
    #[default]
    None,
    Once,
    Repeat,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Notifications {
    pub discord_webhook_url: String,
    pub discord_user_id: String,
    pub notify_on_fail_or_change_map: bool,
    pub notify_on_rune_appear: bool,
    pub notify_on_elite_boss_appear: bool,
    pub notify_on_player_die: bool,
    pub notify_on_player_guildie_appear: bool,
    pub notify_on_player_stranger_appear: bool,
    pub notify_on_player_friend_appear: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(skip_serializing, default)]
    pub id: Option<i64>,
    pub capture_mode: CaptureMode,
    #[serde(default = "enable_rune_solving_default")]
    pub enable_rune_solving: bool,
    pub enable_panic_mode: bool,
    pub stop_on_fail_or_change_map: bool,
    #[serde(default, deserialize_with = "deserialize_with_ok_or_default")]
    pub cycle_run_stop: CycleRunStopMode,
    #[serde(default = "cycle_run_duration_millis_default")]
    pub cycle_run_duration_millis: u64,
    #[serde(default = "cycle_stop_duration_millis_default")]
    pub cycle_stop_duration_millis: u64,
    pub input_method: InputMethod,
    pub input_method_rpc_server_url: String,
    pub notifications: Notifications,
    pub familiars: Familiars,
    #[serde(default = "toggle_actions_key_default")]
    pub toggle_actions_key: KeyBindingConfiguration,
    #[serde(default = "platform_start_key_default")]
    pub platform_start_key: KeyBindingConfiguration,
    #[serde(default = "platform_end_key_default")]
    pub platform_end_key: KeyBindingConfiguration,
    #[serde(default = "platform_add_key_default")]
    pub platform_add_key: KeyBindingConfiguration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            id: None,
            capture_mode: CaptureMode::default(),
            enable_rune_solving: enable_rune_solving_default(),
            enable_panic_mode: false,
            input_method: InputMethod::default(),
            input_method_rpc_server_url: String::default(),
            stop_on_fail_or_change_map: false,
            cycle_run_stop: CycleRunStopMode::default(),
            cycle_run_duration_millis: cycle_run_duration_millis_default(),
            cycle_stop_duration_millis: cycle_stop_duration_millis_default(),
            notifications: Notifications::default(),
            familiars: Familiars::default(),
            toggle_actions_key: toggle_actions_key_default(),
            platform_start_key: platform_start_key_default(),
            platform_end_key: platform_end_key_default(),
            platform_add_key: platform_add_key_default(),
        }
    }
}

impl_identifiable!(Settings);

fn cycle_run_duration_millis_default() -> u64 {
    14400000 // 4 hours
}

fn cycle_stop_duration_millis_default() -> u64 {
    3600000 // 1 hour
}

fn enable_rune_solving_default() -> bool {
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
pub enum CaptureMode {
    #[default]
    BitBlt,
    #[strum(to_string = "Windows 10 (1903 and up)")] // Thanks OBS
    WindowsGraphicsCapture,
    BitBltArea,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Character {
    #[serde(skip_serializing, default)]
    pub id: Option<i64>,
    pub name: String,
    pub ropelift_key: Option<KeyBindingConfiguration>,
    pub teleport_key: Option<KeyBindingConfiguration>,
    #[serde(default = "jump_key_default")]
    pub jump_key: KeyBindingConfiguration,
    pub up_jump_key: Option<KeyBindingConfiguration>,
    #[serde(default = "key_default")]
    pub interact_key: KeyBindingConfiguration,
    #[serde(default = "key_default")]
    pub cash_shop_key: KeyBindingConfiguration,
    #[serde(default = "key_default")]
    pub familiar_menu_key: KeyBindingConfiguration,
    #[serde(default = "key_default")]
    pub to_town_key: KeyBindingConfiguration,
    #[serde(default = "key_default")]
    pub change_channel_key: KeyBindingConfiguration,
    pub feed_pet_key: KeyBindingConfiguration,
    pub feed_pet_millis: u64,
    #[serde(default = "num_pets_default")]
    pub num_pets: u32,
    pub potion_key: KeyBindingConfiguration,
    pub potion_mode: PotionMode,
    pub health_update_millis: u64,
    pub familiar_buff_key: KeyBindingConfiguration,
    #[serde(default = "key_default")]
    pub familiar_essence_key: KeyBindingConfiguration,
    pub sayram_elixir_key: KeyBindingConfiguration,
    pub aurelia_elixir_key: KeyBindingConfiguration,
    pub exp_x3_key: KeyBindingConfiguration,
    pub bonus_exp_key: KeyBindingConfiguration,
    pub legion_wealth_key: KeyBindingConfiguration,
    pub legion_luck_key: KeyBindingConfiguration,
    pub wealth_acquisition_potion_key: KeyBindingConfiguration,
    pub exp_accumulation_potion_key: KeyBindingConfiguration,
    pub extreme_red_potion_key: KeyBindingConfiguration,
    pub extreme_blue_potion_key: KeyBindingConfiguration,
    pub extreme_green_potion_key: KeyBindingConfiguration,
    pub extreme_gold_potion_key: KeyBindingConfiguration,
    pub class: Class,
    pub disable_adjusting: bool,
    pub actions: Vec<ActionConfiguration>,
    #[serde(default, deserialize_with = "deserialize_with_ok_or_default")]
    pub elite_boss_behavior: EliteBossBehavior,
    #[serde(default)]
    pub elite_boss_behavior_key: KeyBinding,
}

fn num_pets_default() -> u32 {
    3
}

fn jump_key_default() -> KeyBindingConfiguration {
    // Enabled is not neccessary but for semantic purpose
    KeyBindingConfiguration {
        key: KeyBinding::Space,
        enabled: true,
    }
}

fn key_default() -> KeyBindingConfiguration {
    // Enabled is not neccessary but for semantic purpose
    KeyBindingConfiguration {
        key: KeyBinding::default(),
        enabled: true,
    }
}

impl Default for Character {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            ropelift_key: None,
            teleport_key: None,
            jump_key: jump_key_default(),
            up_jump_key: None,
            interact_key: key_default(),
            cash_shop_key: key_default(),
            familiar_menu_key: key_default(),
            to_town_key: key_default(),
            change_channel_key: key_default(),
            feed_pet_key: KeyBindingConfiguration::default(),
            feed_pet_millis: 320000,
            num_pets: num_pets_default(),
            potion_key: KeyBindingConfiguration::default(),
            potion_mode: PotionMode::EveryMillis(180000),
            health_update_millis: 1000,
            familiar_buff_key: KeyBindingConfiguration::default(),
            familiar_essence_key: key_default(),
            sayram_elixir_key: KeyBindingConfiguration::default(),
            aurelia_elixir_key: KeyBindingConfiguration::default(),
            exp_x3_key: KeyBindingConfiguration::default(),
            bonus_exp_key: KeyBindingConfiguration::default(),
            legion_wealth_key: KeyBindingConfiguration::default(),
            legion_luck_key: KeyBindingConfiguration::default(),
            wealth_acquisition_potion_key: KeyBindingConfiguration::default(),
            exp_accumulation_potion_key: KeyBindingConfiguration::default(),
            extreme_red_potion_key: KeyBindingConfiguration::default(),
            extreme_blue_potion_key: KeyBindingConfiguration::default(),
            extreme_green_potion_key: KeyBindingConfiguration::default(),
            extreme_gold_potion_key: KeyBindingConfiguration::default(),
            class: Class::default(),
            disable_adjusting: false,
            actions: vec![],
            elite_boss_behavior_key: KeyBinding::default(),
            elite_boss_behavior: EliteBossBehavior::default(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, EnumIter, Display, EnumString)]
pub enum PotionMode {
    EveryMillis(u64),
    Percentage(f32),
}

impl Default for PotionMode {
    fn default() -> Self {
        Self::EveryMillis(0)
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, EnumIter, Display, EnumString)]
pub enum ActionConfigurationCondition {
    EveryMillis(u64),
    Linked,
}

impl Default for ActionConfigurationCondition {
    fn default() -> Self {
        ActionConfigurationCondition::EveryMillis(180000)
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct ActionConfiguration {
    pub key: KeyBinding,
    pub link_key: Option<LinkKeyBinding>,
    pub count: u32,
    pub condition: ActionConfigurationCondition,
    pub with: ActionKeyWith,
    pub wait_before_millis: u64,
    pub wait_before_millis_random_range: u64,
    pub wait_after_millis: u64,
    pub wait_after_millis_random_range: u64,
    pub enabled: bool,
}

impl Default for ActionConfiguration {
    fn default() -> Self {
        // Template for a buff
        Self {
            key: KeyBinding::default(),
            link_key: None,
            count: key_count_default(),
            condition: ActionConfigurationCondition::default(),
            with: ActionKeyWith::Stationary,
            wait_before_millis: 500,
            wait_before_millis_random_range: 0,
            wait_after_millis: 500,
            wait_after_millis_random_range: 0,
            enabled: false,
        }
    }
}

impl From<ActionConfiguration> for Action {
    fn from(value: ActionConfiguration) -> Self {
        Self::Key(ActionKey {
            key: value.key,
            link_key: value.link_key,
            count: value.count,
            position: None,
            condition: match value.condition {
                ActionConfigurationCondition::EveryMillis(millis) => {
                    ActionCondition::EveryMillis(millis)
                }
                ActionConfigurationCondition::Linked => ActionCondition::Linked,
            },
            direction: ActionKeyDirection::Any,
            with: value.with,
            queue_to_front: Some(true),
            wait_before_use_millis: value.wait_before_millis,
            wait_before_use_millis_random_range: value.wait_before_millis_random_range,
            wait_after_use_millis: value.wait_after_millis,
            wait_after_use_millis_random_range: value.wait_after_millis_random_range,
        })
    }
}

#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize)]
pub struct KeyBindingConfiguration {
    pub key: KeyBinding,
    pub enabled: bool,
}

#[derive(Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize)]
pub struct Bound {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

// TODO: Should be part of auto-mobbing or ping-pong logics, not here
impl From<Bound> for Rect {
    fn from(value: Bound) -> Self {
        Self::new(value.x, value.y, value.width, value.height)
    }
}

impl From<Rect> for Bound {
    fn from(value: Rect) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct MobbingKey {
    pub key: KeyBinding,
    pub link_key: Option<LinkKeyBinding>,
    #[serde(default = "key_count_default")]
    pub count: u32,
    pub with: ActionKeyWith,
    pub wait_before_millis: u64,
    pub wait_before_millis_random_range: u64,
    pub wait_after_millis: u64,
    pub wait_after_millis_random_range: u64,
}

impl Default for MobbingKey {
    fn default() -> Self {
        Self {
            key: KeyBinding::default(),
            link_key: None,
            count: key_count_default(),
            with: ActionKeyWith::default(),
            wait_before_millis: 0,
            wait_before_millis_random_range: 0,
            wait_after_millis: 0,
            wait_after_millis_random_range: 0,
        }
    }
}

fn key_count_default() -> u32 {
    1
}

#[derive(
    Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum RotationMode {
    StartToEnd,
    #[default]
    StartToEndThenReverse,
    AutoMobbing,
    PingPong,
}

impl_identifiable!(Character);

#[derive(PartialEq, Clone, Debug, Default, Serialize, Deserialize)]
pub struct Minimap {
    #[serde(skip_serializing)]
    pub id: Option<i64>,
    pub name: String,
    pub width: i32,
    pub height: i32,
    #[serde(default, deserialize_with = "deserialize_with_ok_or_default")]
    pub rotation_mode: RotationMode,
    #[serde(default)]
    pub rotation_ping_pong_bound: Bound,
    #[serde(default)]
    pub rotation_auto_mob_bound: Bound,
    #[serde(default)]
    pub rotation_mobbing_key: MobbingKey,
    pub platforms: Vec<Platform>,
    pub rune_platforms_pathing: bool,
    pub rune_platforms_pathing_up_jump_only: bool,
    pub auto_mob_platforms_pathing: bool,
    pub auto_mob_platforms_pathing_up_jump_only: bool,
    pub auto_mob_platforms_bound: bool,
    pub actions_any_reset_on_erda_condition: bool,
    pub actions: HashMap<String, Vec<Action>>,
    // Not FK, loose coupling to another navigation paths and its index
    #[serde(default)]
    pub paths_id_index: Option<(i64, usize)>,
}

impl_identifiable!(Minimap);

#[derive(PartialEq, Clone, Debug, Default, Serialize, Deserialize)]
pub struct NavigationPaths {
    #[serde(skip_serializing, default)]
    pub id: Option<i64>,
    pub name: String,
    pub paths: Vec<NavigationPath>,
}

impl_identifiable!(NavigationPaths);

#[derive(PartialEq, Clone, Debug, Default, Serialize, Deserialize)]
pub struct NavigationPath {
    pub minimap_snapshot_base64: String,
    pub name_snapshot_base64: String,
    pub name_snapshot_width: i32,
    pub name_snapshot_height: i32,
    pub points: Vec<NavigationPoint>,
}

#[derive(PartialEq, Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct NavigationPoint {
    // Not FK, loose coupling to another navigation paths and its index
    pub next_paths_id_index: Option<(i64, usize)>,
    pub x: i32,
    pub y: i32,
    pub transition: NavigationTransition,
}

#[derive(
    Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum NavigationTransition {
    #[default]
    Portal,
}

fn deserialize_with_ok_or_default<'a, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'a> + Default,
    D: Deserializer<'a>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Platform {
    pub x_start: i32,
    pub x_end: i32,
    pub y: i32,
}

// TODO: Should be part of pathing logics, not here
impl From<Platform> for pathing::Platform {
    fn from(value: Platform) -> Self {
        Self::new(value.x_start..value.x_end, value.y)
    }
}

#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize)]
pub struct Position {
    pub x: i32,
    pub x_random_range: i32,
    pub y: i32,
    pub allow_adjusting: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize)]
pub struct ActionMove {
    pub position: Position,
    pub condition: ActionCondition,
    pub wait_after_move_millis: u64,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct ActionKey {
    pub key: KeyBinding,
    pub link_key: Option<LinkKeyBinding>,
    #[serde(default = "count_default")]
    pub count: u32,
    pub position: Option<Position>,
    pub condition: ActionCondition,
    pub direction: ActionKeyDirection,
    pub with: ActionKeyWith,
    pub wait_before_use_millis: u64,
    pub wait_before_use_millis_random_range: u64,
    pub wait_after_use_millis: u64,
    pub wait_after_use_millis_random_range: u64,
    pub queue_to_front: Option<bool>,
}

impl Default for ActionKey {
    fn default() -> Self {
        Self {
            key: KeyBinding::default(),
            link_key: None,
            count: count_default(),
            position: None,
            condition: ActionCondition::default(),
            direction: ActionKeyDirection::default(),
            with: ActionKeyWith::default(),
            wait_before_use_millis: 0,
            wait_before_use_millis_random_range: 0,
            wait_after_use_millis: 0,
            wait_after_use_millis_random_range: 0,
            queue_to_front: None,
        }
    }
}

#[derive(Clone, Copy, Display, EnumString, EnumIter, PartialEq, Debug, Serialize, Deserialize)]
pub enum LinkKeyBinding {
    Before(KeyBinding),
    AtTheSame(KeyBinding),
    After(KeyBinding),
    Along(KeyBinding),
}

impl LinkKeyBinding {
    pub fn key(&self) -> KeyBinding {
        match self {
            LinkKeyBinding::Before(key)
            | LinkKeyBinding::AtTheSame(key)
            | LinkKeyBinding::After(key)
            | LinkKeyBinding::Along(key) => *key,
        }
    }

    pub fn with_key(&self, key: KeyBinding) -> Self {
        match self {
            LinkKeyBinding::Before(_) => LinkKeyBinding::Before(key),
            LinkKeyBinding::AtTheSame(_) => LinkKeyBinding::AtTheSame(key),
            LinkKeyBinding::After(_) => LinkKeyBinding::After(key),
            LinkKeyBinding::Along(_) => LinkKeyBinding::Along(key),
        }
    }
}

impl Default for LinkKeyBinding {
    fn default() -> Self {
        LinkKeyBinding::Before(KeyBinding::default())
    }
}

fn count_default() -> u32 {
    1
}

#[derive(
    Clone, Copy, Display, Default, EnumString, EnumIter, PartialEq, Debug, Serialize, Deserialize,
)]
pub enum Class {
    Cadena,
    Blaster,
    Ark,
    #[default]
    Generic,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, EnumIter, Display, EnumString)]
pub enum Action {
    Move(ActionMove),
    Key(ActionKey),
}

impl Action {
    pub fn condition(&self) -> ActionCondition {
        match self {
            Action::Move(action) => action.condition,
            Action::Key(action) => action.condition,
        }
    }

    pub fn with_condition(&self, condition: ActionCondition) -> Action {
        match self {
            Action::Move(action) => Action::Move(ActionMove {
                condition,
                ..*action
            }),
            Action::Key(action) => Action::Key(ActionKey {
                condition,
                ..*action
            }),
        }
    }
}

#[derive(
    Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum ActionCondition {
    #[default]
    Any,
    EveryMillis(u64),
    ErdaShowerOffCooldown,
    Linked,
}

#[derive(
    Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum ActionKeyWith {
    #[default]
    Any,
    Stationary,
    DoubleJump,
}

#[derive(
    Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum ActionKeyDirection {
    #[default]
    Any,
    Left,
    Right,
}

#[derive(
    Clone, Copy, PartialEq, Default, Debug, Serialize, Deserialize, EnumIter, Display, EnumString,
)]
pub enum KeyBinding {
    #[default]
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Zero,
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Enter,
    Space,
    Tilde,
    Quote,
    Semicolon,
    Comma,
    Period,
    Slash,
    Esc,
    Shift,
    Ctrl,
    Alt,
}

impl From<KeyKind> for KeyBinding {
    fn from(value: KeyKind) -> Self {
        match value {
            KeyKind::A => KeyBinding::A,
            KeyKind::B => KeyBinding::B,
            KeyKind::C => KeyBinding::C,
            KeyKind::D => KeyBinding::D,
            KeyKind::E => KeyBinding::E,
            KeyKind::F => KeyBinding::F,
            KeyKind::G => KeyBinding::G,
            KeyKind::H => KeyBinding::H,
            KeyKind::I => KeyBinding::I,
            KeyKind::J => KeyBinding::J,
            KeyKind::K => KeyBinding::K,
            KeyKind::L => KeyBinding::L,
            KeyKind::M => KeyBinding::M,
            KeyKind::N => KeyBinding::N,
            KeyKind::O => KeyBinding::O,
            KeyKind::P => KeyBinding::P,
            KeyKind::Q => KeyBinding::Q,
            KeyKind::R => KeyBinding::R,
            KeyKind::S => KeyBinding::S,
            KeyKind::T => KeyBinding::T,
            KeyKind::U => KeyBinding::U,
            KeyKind::V => KeyBinding::V,
            KeyKind::W => KeyBinding::W,
            KeyKind::X => KeyBinding::X,
            KeyKind::Y => KeyBinding::Y,
            KeyKind::Z => KeyBinding::Z,
            KeyKind::Zero => KeyBinding::Zero,
            KeyKind::One => KeyBinding::One,
            KeyKind::Two => KeyBinding::Two,
            KeyKind::Three => KeyBinding::Three,
            KeyKind::Four => KeyBinding::Four,
            KeyKind::Five => KeyBinding::Five,
            KeyKind::Six => KeyBinding::Six,
            KeyKind::Seven => KeyBinding::Seven,
            KeyKind::Eight => KeyBinding::Eight,
            KeyKind::Nine => KeyBinding::Nine,
            KeyKind::F1 => KeyBinding::F1,
            KeyKind::F2 => KeyBinding::F2,
            KeyKind::F3 => KeyBinding::F3,
            KeyKind::F4 => KeyBinding::F4,
            KeyKind::F5 => KeyBinding::F5,
            KeyKind::F6 => KeyBinding::F6,
            KeyKind::F7 => KeyBinding::F7,
            KeyKind::F8 => KeyBinding::F8,
            KeyKind::F9 => KeyBinding::F9,
            KeyKind::F10 => KeyBinding::F10,
            KeyKind::F11 => KeyBinding::F11,
            KeyKind::F12 => KeyBinding::F12,
            KeyKind::Up => KeyBinding::Up,
            KeyKind::Down => KeyBinding::Down,
            KeyKind::Left => KeyBinding::Left,
            KeyKind::Right => KeyBinding::Right,
            KeyKind::Home => KeyBinding::Home,
            KeyKind::End => KeyBinding::End,
            KeyKind::PageUp => KeyBinding::PageUp,
            KeyKind::PageDown => KeyBinding::PageDown,
            KeyKind::Insert => KeyBinding::Insert,
            KeyKind::Delete => KeyBinding::Delete,
            KeyKind::Enter => KeyBinding::Enter,
            KeyKind::Space => KeyBinding::Space,
            KeyKind::Tilde => KeyBinding::Tilde,
            KeyKind::Quote => KeyBinding::Quote,
            KeyKind::Semicolon => KeyBinding::Semicolon,
            KeyKind::Comma => KeyBinding::Comma,
            KeyKind::Period => KeyBinding::Period,
            KeyKind::Slash => KeyBinding::Slash,
            KeyKind::Esc => KeyBinding::Esc,
            KeyKind::Shift => KeyBinding::Shift,
            KeyKind::Ctrl => KeyBinding::Ctrl,
            KeyKind::Alt => KeyBinding::Alt,
        }
    }
}

pub fn database_event_receiver() -> Receiver<DatabaseEvent> {
    EVENT.subscribe()
}

pub fn query_seeds() -> Seeds {
    let mut seeds = query_from_table::<Seeds>(SEEDS)
        .unwrap()
        .into_iter()
        .next()
        .unwrap_or_default();
    if seeds.id.is_none() {
        upsert_to_table(SEEDS, &mut seeds).unwrap();
    }
    seeds
}

pub fn query_settings() -> Settings {
    let mut settings = query_from_table::<Settings>(SETTINGS)
        .unwrap()
        .into_iter()
        .next()
        .unwrap_or_default();
    if settings.id.is_none() {
        upsert_settings(&mut settings).unwrap();
    }
    settings
}

pub fn upsert_settings(settings: &mut Settings) -> Result<()> {
    upsert_to_table(SETTINGS, settings).inspect(|_| {
        let _ = EVENT.send(DatabaseEvent::SettingsUpdated(settings.clone()));
    })
}

pub fn query_characters() -> Result<Vec<Character>> {
    query_from_table(CHARACTERS)
}

pub fn upsert_character(character: &mut Character) -> Result<()> {
    upsert_to_table(CHARACTERS, character).inspect(|_| {
        let _ = EVENT.send(DatabaseEvent::CharacterUpdated(character.clone()));
    })
}

pub fn delete_character(character: &Character) -> Result<()> {
    delete_from_table(CHARACTERS, character).inspect(|_| {
        let _ = EVENT.send(DatabaseEvent::MinimapDeleted(
            character.id.expect("valid id if deleted"),
        ));
    })
}

pub fn query_minimaps() -> Result<Vec<Minimap>> {
    query_from_table(MAPS)
}

pub fn upsert_minimap(minimap: &mut Minimap) -> Result<()> {
    upsert_to_table(MAPS, minimap).inspect(|_| {
        let _ = EVENT.send(DatabaseEvent::MinimapUpdated(minimap.clone()));
    })
}

pub fn delete_minimap(minimap: &Minimap) -> Result<()> {
    delete_from_table(MAPS, minimap).inspect(|_| {
        let _ = EVENT.send(DatabaseEvent::MinimapDeleted(
            minimap.id.expect("valid id if deleted"),
        ));
    })
}

pub fn query_navigation_paths() -> Result<Vec<NavigationPaths>> {
    query_from_table(NAVIGATION_PATHS)
}

pub fn upsert_navigation_paths(paths: &mut NavigationPaths) -> Result<()> {
    upsert_to_table(NAVIGATION_PATHS, paths).inspect(|_| {
        let _ = EVENT.send(DatabaseEvent::NavigationPathsUpdated);
    })
}

pub fn delete_navigation_paths(paths: &NavigationPaths) -> Result<()> {
    delete_from_table(NAVIGATION_PATHS, paths).inspect(|_| {
        let _ = EVENT.send(DatabaseEvent::NavigationPathsDeleted);
    })
}

fn map_data<T>(mut stmt: Statement<'_>, params: impl Params) -> Result<Vec<T>>
where
    T: DeserializeOwned + Identifiable + Default,
{
    Ok(stmt
        .query_map::<T, _, _>(params, |row| {
            let id = row.get::<_, i64>(0).unwrap();
            let data = row.get::<_, String>(1).unwrap();
            let mut value = serde_json::from_str::<'_, T>(data.as_str()).unwrap_or_default();
            value.set_id(id);
            Ok(value)
        })?
        .filter_map(|c| c.ok())
        .collect::<Vec<_>>())
}

fn query_from_table<T>(table: &str) -> Result<Vec<T>>
where
    T: DeserializeOwned + Identifiable + Default,
{
    let conn = CONNECTION.lock().unwrap();
    let stmt = format!("SELECT id, data FROM {table};");
    let stmt = conn.prepare(&stmt).unwrap();
    map_data(stmt, [])
}

fn upsert_to_table<T>(table: &str, data: &mut T) -> Result<()>
where
    T: Serialize + Identifiable,
{
    let json = serde_json::to_string(&data).unwrap();
    let conn = CONNECTION.lock().unwrap();
    let stmt = format!(
        "INSERT INTO {table} (id, data) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET data = ?2;",
    );
    match data.id() {
        Some(id) => {
            if conn.execute(&stmt, (id, &json))? > 0 {
                Ok(())
            } else {
                bail!("no row was updated")
            }
        }
        None => {
            if conn.execute(&stmt, (Null, &json))? > 0 {
                data.set_id(conn.last_insert_rowid());
                Ok(())
            } else {
                bail!("no row was inserted")
            }
        }
    }
}

fn delete_from_table<T: Identifiable>(table: &str, data: &T) -> Result<()> {
    fn inner(table: &str, id: Option<i64>) -> Result<()> {
        if id.is_some() {
            let conn = CONNECTION.lock().unwrap();
            let stmt = format!("DELETE FROM {table} WHERE id = ?1;");
            let deleted = conn.execute(&stmt, [id.unwrap()])?;

            if deleted > 0 {
                return Ok(());
            }
        }
        bail!("no row was deleted")
    }

    inner(table, data.id())
}
