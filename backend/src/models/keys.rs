use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};

use super::{ActionKeyWith, deserialize_with_ok_or_default};

#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize)]
pub struct KeyBindingConfiguration {
    pub key: KeyBinding,
    #[serde(default)]
    pub enabled: bool,
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
    Backspace,
}

#[derive(
    Clone, Copy, Display, EnumString, EnumIter, PartialEq, Debug, Serialize, Deserialize, Default,
)]
pub enum LinkKeyBinding {
    #[default]
    None,
    Before(KeyBinding),
    AtTheSame(KeyBinding),
    After(KeyBinding),
    Along(KeyBinding),
}

impl LinkKeyBinding {
    pub fn key(&self) -> Option<KeyBinding> {
        match self {
            LinkKeyBinding::Before(key)
            | LinkKeyBinding::AtTheSame(key)
            | LinkKeyBinding::After(key)
            | LinkKeyBinding::Along(key) => Some(*key),
            LinkKeyBinding::None => None,
        }
    }

    pub fn with_key(&self, key: KeyBinding) -> Self {
        match self {
            LinkKeyBinding::Before(_) => LinkKeyBinding::Before(key),
            LinkKeyBinding::AtTheSame(_) => LinkKeyBinding::AtTheSame(key),
            LinkKeyBinding::After(_) => LinkKeyBinding::After(key),
            LinkKeyBinding::Along(_) => LinkKeyBinding::Along(key),
            LinkKeyBinding::None => LinkKeyBinding::None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct MobbingKey {
    pub key: KeyBinding,
    #[serde(default)]
    pub key_hold_millis: u64,
    #[serde(default, deserialize_with = "deserialize_with_ok_or_default")]
    pub link_key: LinkKeyBinding,
    #[serde(default = "count_default")]
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
            key_hold_millis: 0,
            link_key: LinkKeyBinding::None,
            count: count_default(),
            with: ActionKeyWith::default(),
            wait_before_millis: 0,
            wait_before_millis_random_range: 0,
            wait_after_millis: 0,
            wait_after_millis_random_range: 0,
        }
    }
}

fn count_default() -> u32 {
    1
}
