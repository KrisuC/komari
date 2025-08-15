use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::Entry},
    fmt::Debug,
};

use anyhow::{Result, bail};
#[cfg(test)]
use mockall::automock;
#[cfg(windows)]
use platforms::capture::WindowsCaptureKind;
use platforms::{
    CoordinateRelative, Error, Window,
    capture::{Capture as PlatformCapture, Frame},
    input::{
        Input as PlatformInput, InputKind as PlatformInputKind,
        InputReceiver as PlatformInputReceiver, KeyKind as PlatformKeyKind,
        KeyState as PlatformKeyState, MouseKind as PlatformMouseKind,
    },
};

use crate::{
    CaptureMode, KeyBinding,
    context::{MS_PER_TICK, MS_PER_TICK_F32},
    database::Seeds,
    rng::Rng,
    rpc::{
        Coordinate as RpcCoordinate, InputService, Key as RpcKeyKind, KeyState as RpcKeyState,
        MouseAction as RpcMouseKind,
    },
};

/// Base mean in milliseconds to generate a pair from.
const BASE_MEAN_MS_DELAY: f32 = 100.0;

/// Base standard deviation in milliseconds to generate a pair from.
const BASE_STD_MS_DELAY: f32 = 20.0;

/// The rate at which generated standard deviation will revert to the base [`BASE_STD_MS_DELAY`]
/// over time.
const MEAN_STD_REVERSION_RATE: f32 = 0.2;

/// The rate at which generated mean will revert to the base [`BASE_MEAN_MS_DELAY`] over time.
const MEAN_STD_VOLATILITY: f32 = 3.0;

/// The current of key state.
///
/// This is a bridge enum between platform-specific and gRPC.
#[derive(Debug)]
pub enum KeyState {
    Pressed,
    Released,
}

impl From<PlatformKeyState> for KeyState {
    fn from(value: PlatformKeyState) -> Self {
        match value {
            PlatformKeyState::Pressed => KeyState::Pressed,
            PlatformKeyState::Released => KeyState::Released,
        }
    }
}

impl From<RpcKeyState> for KeyState {
    fn from(value: RpcKeyState) -> Self {
        match value {
            RpcKeyState::Pressed => KeyState::Pressed,
            RpcKeyState::Released => KeyState::Released,
        }
    }
}

/// The kind of mouse movement/action to perform.
///
/// This is a bridge enum between platform-specific and gRPC.
#[derive(Debug)]
pub enum MouseKind {
    Move,
    Click,
    Scroll,
}

impl From<MouseKind> for RpcMouseKind {
    fn from(value: MouseKind) -> Self {
        match value {
            MouseKind::Move => RpcMouseKind::Move,
            MouseKind::Click => RpcMouseKind::Click,
            MouseKind::Scroll => RpcMouseKind::ScrollDown,
        }
    }
}

impl From<MouseKind> for PlatformMouseKind {
    fn from(value: MouseKind) -> Self {
        match value {
            MouseKind::Move => PlatformMouseKind::Move,
            MouseKind::Click => PlatformMouseKind::Click,
            MouseKind::Scroll => PlatformMouseKind::Scroll,
        }
    }
}

/// The kind of key to sent.
///
/// This is a bridge enum between platform-specific, gRPC and database.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum KeyKind {
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
    Ctrl,
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
    Alt,
}

impl From<KeyBinding> for KeyKind {
    fn from(value: KeyBinding) -> Self {
        match value {
            KeyBinding::A => KeyKind::A,
            KeyBinding::B => KeyKind::B,
            KeyBinding::C => KeyKind::C,
            KeyBinding::D => KeyKind::D,
            KeyBinding::E => KeyKind::E,
            KeyBinding::F => KeyKind::F,
            KeyBinding::G => KeyKind::G,
            KeyBinding::H => KeyKind::H,
            KeyBinding::I => KeyKind::I,
            KeyBinding::J => KeyKind::J,
            KeyBinding::K => KeyKind::K,
            KeyBinding::L => KeyKind::L,
            KeyBinding::M => KeyKind::M,
            KeyBinding::N => KeyKind::N,
            KeyBinding::O => KeyKind::O,
            KeyBinding::P => KeyKind::P,
            KeyBinding::Q => KeyKind::Q,
            KeyBinding::R => KeyKind::R,
            KeyBinding::S => KeyKind::S,
            KeyBinding::T => KeyKind::T,
            KeyBinding::U => KeyKind::U,
            KeyBinding::V => KeyKind::V,
            KeyBinding::W => KeyKind::W,
            KeyBinding::X => KeyKind::X,
            KeyBinding::Y => KeyKind::Y,
            KeyBinding::Z => KeyKind::Z,
            KeyBinding::Zero => KeyKind::Zero,
            KeyBinding::One => KeyKind::One,
            KeyBinding::Two => KeyKind::Two,
            KeyBinding::Three => KeyKind::Three,
            KeyBinding::Four => KeyKind::Four,
            KeyBinding::Five => KeyKind::Five,
            KeyBinding::Six => KeyKind::Six,
            KeyBinding::Seven => KeyKind::Seven,
            KeyBinding::Eight => KeyKind::Eight,
            KeyBinding::Nine => KeyKind::Nine,
            KeyBinding::F1 => KeyKind::F1,
            KeyBinding::F2 => KeyKind::F2,
            KeyBinding::F3 => KeyKind::F3,
            KeyBinding::F4 => KeyKind::F4,
            KeyBinding::F5 => KeyKind::F5,
            KeyBinding::F6 => KeyKind::F6,
            KeyBinding::F7 => KeyKind::F7,
            KeyBinding::F8 => KeyKind::F8,
            KeyBinding::F9 => KeyKind::F9,
            KeyBinding::F10 => KeyKind::F10,
            KeyBinding::F11 => KeyKind::F11,
            KeyBinding::F12 => KeyKind::F12,
            KeyBinding::Up => KeyKind::Up,
            KeyBinding::Down => KeyKind::Down,
            KeyBinding::Left => KeyKind::Left,
            KeyBinding::Right => KeyKind::Right,
            KeyBinding::Home => KeyKind::Home,
            KeyBinding::End => KeyKind::End,
            KeyBinding::PageUp => KeyKind::PageUp,
            KeyBinding::PageDown => KeyKind::PageDown,
            KeyBinding::Insert => KeyKind::Insert,
            KeyBinding::Delete => KeyKind::Delete,
            KeyBinding::Enter => KeyKind::Enter,
            KeyBinding::Space => KeyKind::Space,
            KeyBinding::Tilde => KeyKind::Tilde,
            KeyBinding::Quote => KeyKind::Quote,
            KeyBinding::Semicolon => KeyKind::Semicolon,
            KeyBinding::Comma => KeyKind::Comma,
            KeyBinding::Period => KeyKind::Period,
            KeyBinding::Slash => KeyKind::Slash,
            KeyBinding::Esc => KeyKind::Esc,
            KeyBinding::Shift => KeyKind::Shift,
            KeyBinding::Ctrl => KeyKind::Ctrl,
            KeyBinding::Alt => KeyKind::Alt,
        }
    }
}

impl From<PlatformKeyKind> for KeyKind {
    fn from(value: PlatformKeyKind) -> Self {
        match value {
            PlatformKeyKind::A => KeyKind::A,
            PlatformKeyKind::B => KeyKind::B,
            PlatformKeyKind::C => KeyKind::C,
            PlatformKeyKind::D => KeyKind::D,
            PlatformKeyKind::E => KeyKind::E,
            PlatformKeyKind::F => KeyKind::F,
            PlatformKeyKind::G => KeyKind::G,
            PlatformKeyKind::H => KeyKind::H,
            PlatformKeyKind::I => KeyKind::I,
            PlatformKeyKind::J => KeyKind::J,
            PlatformKeyKind::K => KeyKind::K,
            PlatformKeyKind::L => KeyKind::L,
            PlatformKeyKind::M => KeyKind::M,
            PlatformKeyKind::N => KeyKind::N,
            PlatformKeyKind::O => KeyKind::O,
            PlatformKeyKind::P => KeyKind::P,
            PlatformKeyKind::Q => KeyKind::Q,
            PlatformKeyKind::R => KeyKind::R,
            PlatformKeyKind::S => KeyKind::S,
            PlatformKeyKind::T => KeyKind::T,
            PlatformKeyKind::U => KeyKind::U,
            PlatformKeyKind::V => KeyKind::V,
            PlatformKeyKind::W => KeyKind::W,
            PlatformKeyKind::X => KeyKind::X,
            PlatformKeyKind::Y => KeyKind::Y,
            PlatformKeyKind::Z => KeyKind::Z,
            PlatformKeyKind::Zero => KeyKind::Zero,
            PlatformKeyKind::One => KeyKind::One,
            PlatformKeyKind::Two => KeyKind::Two,
            PlatformKeyKind::Three => KeyKind::Three,
            PlatformKeyKind::Four => KeyKind::Four,
            PlatformKeyKind::Five => KeyKind::Five,
            PlatformKeyKind::Six => KeyKind::Six,
            PlatformKeyKind::Seven => KeyKind::Seven,
            PlatformKeyKind::Eight => KeyKind::Eight,
            PlatformKeyKind::Nine => KeyKind::Nine,
            PlatformKeyKind::F1 => KeyKind::F1,
            PlatformKeyKind::F2 => KeyKind::F2,
            PlatformKeyKind::F3 => KeyKind::F3,
            PlatformKeyKind::F4 => KeyKind::F4,
            PlatformKeyKind::F5 => KeyKind::F5,
            PlatformKeyKind::F6 => KeyKind::F6,
            PlatformKeyKind::F7 => KeyKind::F7,
            PlatformKeyKind::F8 => KeyKind::F8,
            PlatformKeyKind::F9 => KeyKind::F9,
            PlatformKeyKind::F10 => KeyKind::F10,
            PlatformKeyKind::F11 => KeyKind::F11,
            PlatformKeyKind::F12 => KeyKind::F12,
            PlatformKeyKind::Up => KeyKind::Up,
            PlatformKeyKind::Down => KeyKind::Down,
            PlatformKeyKind::Left => KeyKind::Left,
            PlatformKeyKind::Right => KeyKind::Right,
            PlatformKeyKind::Home => KeyKind::Home,
            PlatformKeyKind::End => KeyKind::End,
            PlatformKeyKind::PageUp => KeyKind::PageUp,
            PlatformKeyKind::PageDown => KeyKind::PageDown,
            PlatformKeyKind::Insert => KeyKind::Insert,
            PlatformKeyKind::Delete => KeyKind::Delete,
            PlatformKeyKind::Ctrl => KeyKind::Ctrl,
            PlatformKeyKind::Enter => KeyKind::Enter,
            PlatformKeyKind::Space => KeyKind::Space,
            PlatformKeyKind::Tilde => KeyKind::Tilde,
            PlatformKeyKind::Quote => KeyKind::Quote,
            PlatformKeyKind::Semicolon => KeyKind::Semicolon,
            PlatformKeyKind::Comma => KeyKind::Comma,
            PlatformKeyKind::Period => KeyKind::Period,
            PlatformKeyKind::Slash => KeyKind::Slash,
            PlatformKeyKind::Esc => KeyKind::Esc,
            PlatformKeyKind::Shift => KeyKind::Shift,
            PlatformKeyKind::Alt => KeyKind::Alt,
        }
    }
}

impl From<KeyKind> for PlatformKeyKind {
    fn from(value: KeyKind) -> Self {
        match value {
            KeyKind::A => PlatformKeyKind::A,
            KeyKind::B => PlatformKeyKind::B,
            KeyKind::C => PlatformKeyKind::C,
            KeyKind::D => PlatformKeyKind::D,
            KeyKind::E => PlatformKeyKind::E,
            KeyKind::F => PlatformKeyKind::F,
            KeyKind::G => PlatformKeyKind::G,
            KeyKind::H => PlatformKeyKind::H,
            KeyKind::I => PlatformKeyKind::I,
            KeyKind::J => PlatformKeyKind::J,
            KeyKind::K => PlatformKeyKind::K,
            KeyKind::L => PlatformKeyKind::L,
            KeyKind::M => PlatformKeyKind::M,
            KeyKind::N => PlatformKeyKind::N,
            KeyKind::O => PlatformKeyKind::O,
            KeyKind::P => PlatformKeyKind::P,
            KeyKind::Q => PlatformKeyKind::Q,
            KeyKind::R => PlatformKeyKind::R,
            KeyKind::S => PlatformKeyKind::S,
            KeyKind::T => PlatformKeyKind::T,
            KeyKind::U => PlatformKeyKind::U,
            KeyKind::V => PlatformKeyKind::V,
            KeyKind::W => PlatformKeyKind::W,
            KeyKind::X => PlatformKeyKind::X,
            KeyKind::Y => PlatformKeyKind::Y,
            KeyKind::Z => PlatformKeyKind::Z,
            KeyKind::Zero => PlatformKeyKind::Zero,
            KeyKind::One => PlatformKeyKind::One,
            KeyKind::Two => PlatformKeyKind::Two,
            KeyKind::Three => PlatformKeyKind::Three,
            KeyKind::Four => PlatformKeyKind::Four,
            KeyKind::Five => PlatformKeyKind::Five,
            KeyKind::Six => PlatformKeyKind::Six,
            KeyKind::Seven => PlatformKeyKind::Seven,
            KeyKind::Eight => PlatformKeyKind::Eight,
            KeyKind::Nine => PlatformKeyKind::Nine,
            KeyKind::F1 => PlatformKeyKind::F1,
            KeyKind::F2 => PlatformKeyKind::F2,
            KeyKind::F3 => PlatformKeyKind::F3,
            KeyKind::F4 => PlatformKeyKind::F4,
            KeyKind::F5 => PlatformKeyKind::F5,
            KeyKind::F6 => PlatformKeyKind::F6,
            KeyKind::F7 => PlatformKeyKind::F7,
            KeyKind::F8 => PlatformKeyKind::F8,
            KeyKind::F9 => PlatformKeyKind::F9,
            KeyKind::F10 => PlatformKeyKind::F10,
            KeyKind::F11 => PlatformKeyKind::F11,
            KeyKind::F12 => PlatformKeyKind::F12,
            KeyKind::Up => PlatformKeyKind::Up,
            KeyKind::Down => PlatformKeyKind::Down,
            KeyKind::Left => PlatformKeyKind::Left,
            KeyKind::Right => PlatformKeyKind::Right,
            KeyKind::Home => PlatformKeyKind::Home,
            KeyKind::End => PlatformKeyKind::End,
            KeyKind::PageUp => PlatformKeyKind::PageUp,
            KeyKind::PageDown => PlatformKeyKind::PageDown,
            KeyKind::Insert => PlatformKeyKind::Insert,
            KeyKind::Delete => PlatformKeyKind::Delete,
            KeyKind::Ctrl => PlatformKeyKind::Ctrl,
            KeyKind::Enter => PlatformKeyKind::Enter,
            KeyKind::Space => PlatformKeyKind::Space,
            KeyKind::Tilde => PlatformKeyKind::Tilde,
            KeyKind::Quote => PlatformKeyKind::Quote,
            KeyKind::Semicolon => PlatformKeyKind::Semicolon,
            KeyKind::Comma => PlatformKeyKind::Comma,
            KeyKind::Period => PlatformKeyKind::Period,
            KeyKind::Slash => PlatformKeyKind::Slash,
            KeyKind::Esc => PlatformKeyKind::Esc,
            KeyKind::Shift => PlatformKeyKind::Shift,
            KeyKind::Alt => PlatformKeyKind::Alt,
        }
    }
}

impl From<KeyKind> for RpcKeyKind {
    fn from(value: KeyKind) -> Self {
        match value {
            KeyKind::A => RpcKeyKind::A,
            KeyKind::B => RpcKeyKind::B,
            KeyKind::C => RpcKeyKind::C,
            KeyKind::D => RpcKeyKind::D,
            KeyKind::E => RpcKeyKind::E,
            KeyKind::F => RpcKeyKind::F,
            KeyKind::G => RpcKeyKind::G,
            KeyKind::H => RpcKeyKind::H,
            KeyKind::I => RpcKeyKind::I,
            KeyKind::J => RpcKeyKind::J,
            KeyKind::K => RpcKeyKind::K,
            KeyKind::L => RpcKeyKind::L,
            KeyKind::M => RpcKeyKind::M,
            KeyKind::N => RpcKeyKind::N,
            KeyKind::O => RpcKeyKind::O,
            KeyKind::P => RpcKeyKind::P,
            KeyKind::Q => RpcKeyKind::Q,
            KeyKind::R => RpcKeyKind::R,
            KeyKind::S => RpcKeyKind::S,
            KeyKind::T => RpcKeyKind::T,
            KeyKind::U => RpcKeyKind::U,
            KeyKind::V => RpcKeyKind::V,
            KeyKind::W => RpcKeyKind::W,
            KeyKind::X => RpcKeyKind::X,
            KeyKind::Y => RpcKeyKind::Y,
            KeyKind::Z => RpcKeyKind::Z,
            KeyKind::Zero => RpcKeyKind::Zero,
            KeyKind::One => RpcKeyKind::One,
            KeyKind::Two => RpcKeyKind::Two,
            KeyKind::Three => RpcKeyKind::Three,
            KeyKind::Four => RpcKeyKind::Four,
            KeyKind::Five => RpcKeyKind::Five,
            KeyKind::Six => RpcKeyKind::Six,
            KeyKind::Seven => RpcKeyKind::Seven,
            KeyKind::Eight => RpcKeyKind::Eight,
            KeyKind::Nine => RpcKeyKind::Nine,
            KeyKind::F1 => RpcKeyKind::F1,
            KeyKind::F2 => RpcKeyKind::F2,
            KeyKind::F3 => RpcKeyKind::F3,
            KeyKind::F4 => RpcKeyKind::F4,
            KeyKind::F5 => RpcKeyKind::F5,
            KeyKind::F6 => RpcKeyKind::F6,
            KeyKind::F7 => RpcKeyKind::F7,
            KeyKind::F8 => RpcKeyKind::F8,
            KeyKind::F9 => RpcKeyKind::F9,
            KeyKind::F10 => RpcKeyKind::F10,
            KeyKind::F11 => RpcKeyKind::F11,
            KeyKind::F12 => RpcKeyKind::F12,
            KeyKind::Up => RpcKeyKind::Up,
            KeyKind::Down => RpcKeyKind::Down,
            KeyKind::Left => RpcKeyKind::Left,
            KeyKind::Right => RpcKeyKind::Right,
            KeyKind::Home => RpcKeyKind::Home,
            KeyKind::End => RpcKeyKind::End,
            KeyKind::PageUp => RpcKeyKind::PageUp,
            KeyKind::PageDown => RpcKeyKind::PageDown,
            KeyKind::Insert => RpcKeyKind::Insert,
            KeyKind::Delete => RpcKeyKind::Delete,
            KeyKind::Ctrl => RpcKeyKind::Ctrl,
            KeyKind::Enter => RpcKeyKind::Enter,
            KeyKind::Space => RpcKeyKind::Space,
            KeyKind::Tilde => RpcKeyKind::Tilde,
            KeyKind::Quote => RpcKeyKind::Quote,
            KeyKind::Semicolon => RpcKeyKind::Semicolon,
            KeyKind::Comma => RpcKeyKind::Comma,
            KeyKind::Period => RpcKeyKind::Period,
            KeyKind::Slash => RpcKeyKind::Slash,
            KeyKind::Esc => RpcKeyKind::Esc,
            KeyKind::Shift => RpcKeyKind::Shift,
            KeyKind::Alt => RpcKeyKind::Alt,
        }
    }
}

/// A receiver to receive to platform keystroke event.
///
/// This is a bridge trait for [`KeyKind`].
#[cfg_attr(test, automock)]
pub trait InputReceiver: Debug + 'static {
    fn set_window_and_input_kind(&mut self, window: Window, kind: PlatformInputKind);

    fn try_recv(&mut self) -> Result<KeyKind>;
}

#[derive(Debug)]
pub struct DefaultInputReceiver {
    inner: PlatformInputReceiver,
}

impl DefaultInputReceiver {
    pub fn new(window: Window, kind: PlatformInputKind) -> Self {
        Self {
            inner: PlatformInputReceiver::new(window, kind).expect("supported platform"),
        }
    }
}

impl InputReceiver for DefaultInputReceiver {
    fn set_window_and_input_kind(&mut self, window: Window, kind: PlatformInputKind) {
        self.inner = PlatformInputReceiver::new(window, kind).expect("supported platform")
    }

    #[inline]
    fn try_recv(&mut self) -> Result<KeyKind> {
        Ok(self.inner.try_recv()?.into())
    }
}

/// Input method to use.
///
/// This is a bridge enum between platform-specific and gRPC input options.
pub enum InputMethod {
    Rpc(Window, String),
    Default(Window, PlatformInputKind),
}

/// Inner kind of [`InputMethod`].
///
/// The above [`InputMethod`] will be converted to this inner kind that contains the actual
/// sending structure.
#[derive(Debug)]
enum InputMethodInner {
    Rpc(Window, Option<RefCell<InputService>>),
    Default(PlatformInput),
}

/// States of input delay tracking.
#[derive(Debug)]
enum InputDelay {
    Untracked,
    Tracked,
    AlreadyTracked,
}

/// A trait for sending inputs.
#[cfg_attr(test, automock)]
pub trait Input: Debug {
    /// Performs a tick update.
    fn update(&mut self, tick: u64);

    /// Overwrites the current input method with new `method`.
    fn set_method(&mut self, method: InputMethod);

    /// Sends mouse `kind` to `(x, y)` relative to the client coordinate (e.g. capture area).
    ///
    /// `(0, 0)` is top-left and `(width, height)` is bottom-right.
    fn send_mouse(&self, x: i32, y: i32, kind: MouseKind) -> Result<()>;

    /// Presses a single key `kind`.
    fn send_key(&self, kind: KeyKind) -> Result<()>;

    /// Releases a held key `kind`.
    fn send_key_up(&self, kind: KeyKind) -> Result<()>;

    /// Holds down key `kind`.
    fn send_key_down(&self, kind: KeyKind) -> Result<()>;

    /// Whether all keys are cleared.
    fn all_keys_cleared(&self) -> bool;
}

/// Default implementation of [`Input`].
#[derive(Debug)]
pub struct DefaultInput {
    kind: InputMethodInner,
    delay_rng: Rng,
    delay_mean_std_pair: (f32, f32),
    delay_map: RefCell<HashMap<KeyKind, (u32, bool)>>,
}

impl DefaultInput {
    pub fn new(method: InputMethod, seeds: Seeds) -> Self {
        Self {
            kind: input_method_inner_from(method, &seeds.seed),
            delay_rng: Rng::new(seeds.seed),
            delay_mean_std_pair: (BASE_MEAN_MS_DELAY, BASE_STD_MS_DELAY),
            delay_map: RefCell::new(HashMap::new()),
        }
    }

    #[inline]
    fn key_state(&self, kind: KeyKind) -> Result<KeyState> {
        match &self.kind {
            InputMethodInner::Rpc(_, service) => {
                if let Some(cell) = service {
                    Ok(cell.borrow_mut().key_state(kind.into())?.into())
                } else {
                    bail!("service not connected")
                }
            }
            InputMethodInner::Default(input) => Ok(input.key_state(kind.into())?.into()),
        }
    }

    #[inline]
    fn send_key_inner(&self, kind: KeyKind) -> Result<()> {
        match &self.kind {
            InputMethodInner::Rpc(_, service) => {
                if let Some(cell) = service {
                    cell.borrow_mut()
                        .send_key(kind.into(), self.random_input_delay_tick_count().0)?;
                }
            }
            InputMethodInner::Default(input) => match self.track_input_delay(kind) {
                InputDelay::Untracked => input.send_key(kind.into())?,
                InputDelay::Tracked => input.send_key_down(kind.into())?,
                InputDelay::AlreadyTracked => (),
            },
        }

        Ok(())
    }

    #[inline]
    fn send_key_up_inner(&self, kind: KeyKind, forced: bool) -> Result<()> {
        match &self.kind {
            InputMethodInner::Rpc(_, service) => {
                if let Some(cell) = service {
                    cell.borrow_mut().send_key_up(kind.into())?;
                }
            }
            InputMethodInner::Default(input) => {
                if forced || !self.has_input_delay(kind) {
                    input.send_key_up(kind.into())?;
                }
            }
        }

        Ok(())
    }

    #[inline]
    fn send_key_down_inner(&self, kind: KeyKind) -> Result<()> {
        match &self.kind {
            InputMethodInner::Rpc(_, service) => {
                if let Some(cell) = service {
                    cell.borrow_mut().send_key_down(kind.into())?;
                }
            }
            InputMethodInner::Default(input) => {
                if !self.has_input_delay(kind) {
                    input.send_key_down(kind.into())?;
                }
            }
        }

        Ok(())
    }

    #[inline]
    fn has_input_delay(&self, kind: KeyKind) -> bool {
        self.delay_map.borrow().contains_key(&kind)
    }

    /// Tracks input delay for a key that is about to be pressed for both down and up key strokes.
    ///
    /// Upon returning [`InputDelay::Tracked`], it is expected that only key down is sent. Later,
    /// it will be automatically released by [`Self::update_input_delay`] once the input delay has
    /// timed out. If [`InputDelay::Untracked`] is returned, it is expected that both down and up
    /// key strokes are sent.
    ///
    /// This function should only be used for [`Self::send_key`] as the other two should be handled
    /// by the external caller.
    fn track_input_delay(&self, kind: KeyKind) -> InputDelay {
        let mut map = self.delay_map.borrow_mut();
        let entry = map.entry(kind);
        if matches!(entry, Entry::Occupied(_)) {
            return InputDelay::AlreadyTracked;
        }

        let (_, delay_tick_count) = self.random_input_delay_tick_count();
        if delay_tick_count == 0 {
            return InputDelay::Untracked;
        }

        let _ = entry.insert_entry((delay_tick_count, false));
        InputDelay::Tracked
    }

    /// Updates the input delay (key up timing) for held down keys and delay std/mean pair.
    #[inline]
    fn update(&mut self, game_tick: u64) {
        const UPDATE_MEAN_STD_PAIR_INTERVAL: u64 = 200;

        if game_tick > 0 && game_tick.is_multiple_of(UPDATE_MEAN_STD_PAIR_INTERVAL) {
            let (mean, std) = self.delay_mean_std_pair;
            self.delay_mean_std_pair = self.delay_rng.random_mean_std_pair(
                BASE_MEAN_MS_DELAY,
                mean,
                BASE_STD_MS_DELAY,
                std,
                MEAN_STD_REVERSION_RATE,
                MEAN_STD_VOLATILITY,
            )
        }

        let mut map = self.delay_map.borrow_mut();
        if map.is_empty() {
            return;
        }
        map.retain(|kind, (delay, did_send_up)| {
            *delay = delay.saturating_sub(1);
            if *delay == 0 {
                if !*did_send_up {
                    *did_send_up = true;
                    let _ = self.send_key_up_inner(*kind, true);
                }

                self.key_state(*kind)
                    .ok()
                    .is_some_and(|state| matches!(state, KeyState::Pressed))
            } else {
                true
            }
        });
    }

    fn random_input_delay_tick_count(&self) -> (f32, u32) {
        let (mean, std) = self.delay_mean_std_pair;
        self.delay_rng
            .random_delay_tick_count(mean, std, MS_PER_TICK_F32, 80.0, 120.0)
    }
}

impl Input for DefaultInput {
    fn update(&mut self, tick: u64) {
        self.update(tick);
    }

    fn set_method(&mut self, method: InputMethod) {
        self.kind = input_method_inner_from(method, self.delay_rng.seed());
    }

    fn send_mouse(&self, x: i32, y: i32, kind: MouseKind) -> Result<()> {
        match &self.kind {
            InputMethodInner::Rpc(window, service) => {
                if let Some(cell) = service {
                    let mut borrow = cell.borrow_mut();
                    let relative = match borrow.mouse_coordinate() {
                        RpcCoordinate::Screen => CoordinateRelative::Monitor,
                        RpcCoordinate::Relative => CoordinateRelative::Window,
                    };
                    let coordinates = window.convert_coordinate(x, y, relative)?;

                    borrow.send_mouse(
                        coordinates.width,
                        coordinates.height,
                        coordinates.x,
                        coordinates.y,
                        kind.into(),
                    )?;
                }
            }
            InputMethodInner::Default(keys) => {
                let kind = match kind {
                    MouseKind::Move => PlatformMouseKind::Move,
                    MouseKind::Click => PlatformMouseKind::Click,
                    MouseKind::Scroll => PlatformMouseKind::Scroll,
                };
                keys.send_mouse(x, y, kind)?;
            }
        }

        Ok(())
    }

    fn send_key(&self, kind: KeyKind) -> Result<()> {
        self.send_key_inner(kind)
    }

    fn send_key_up(&self, kind: KeyKind) -> Result<()> {
        self.send_key_up_inner(kind, false)
    }

    fn send_key_down(&self, kind: KeyKind) -> Result<()> {
        self.send_key_down_inner(kind)
    }

    #[inline]
    fn all_keys_cleared(&self) -> bool {
        self.delay_map.borrow().is_empty()
    }
}

/// A trait for managing different capture modes.
///
/// A bridge trait between platform-specific and database.
#[cfg_attr(test, automock)]
pub trait Capture: Debug + 'static {
    fn grab(&mut self) -> Result<Frame, Error>;

    fn window(&self) -> Window;

    fn set_window(&mut self, window: Window);

    fn mode(&self) -> CaptureMode;

    fn set_mode(&mut self, mode: CaptureMode);
}

#[derive(Debug)]
pub struct DefaultCapture {
    inner: PlatformCapture,
    mode: CaptureMode,
}

impl DefaultCapture {
    pub fn new(window: Window) -> Self {
        Self {
            inner: PlatformCapture::new(window).expect("supported platform"),
            mode: CaptureMode::BitBlt,
        }
    }
}

impl Capture for DefaultCapture {
    #[inline]
    fn grab(&mut self) -> Result<Frame, Error> {
        self.inner.grab()
    }

    #[inline]
    fn window(&self) -> Window {
        self.inner.window().expect("supported platform")
    }

    #[inline]
    fn set_window(&mut self, window: Window) {
        self.inner.set_window(window).expect("supported platform");
    }

    #[inline]
    fn mode(&self) -> CaptureMode {
        self.mode
    }

    #[inline]
    fn set_mode(&mut self, mode: CaptureMode) {
        self.mode = mode;

        if cfg!(windows) {
            let kind = match mode {
                CaptureMode::BitBlt => WindowsCaptureKind::BitBlt,
                CaptureMode::WindowsGraphicsCapture => WindowsCaptureKind::Wgc(MS_PER_TICK),
                CaptureMode::BitBltArea => WindowsCaptureKind::BitBltArea,
            };
            let _ = self.inner.windows_capture_kind(kind);
        }
    }
}

#[inline]
fn input_method_inner_from(method: InputMethod, seed: &[u8]) -> InputMethodInner {
    match method {
        InputMethod::Rpc(handle, url) => {
            let mut service = InputService::connect(url);
            if let Ok(ref mut service) = service {
                let _ = service.init(seed);
            }

            InputMethodInner::Rpc(handle, service.ok().map(RefCell::new))
        }
        InputMethod::Default(handle, kind) => {
            InputMethodInner::Default(PlatformInput::new(handle, kind).expect("supported platform"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use super::*;

    const SEED: [u8; 32] = [
        64, 241, 206, 219, 49, 21, 218, 145, 254, 152, 68, 176, 242, 238, 152, 14, 176, 241, 153,
        64, 44, 192, 172, 191, 191, 157, 107, 206, 193, 55, 115, 68,
    ];

    fn test_key_sender() -> DefaultInput {
        let seeds = Seeds {
            id: None,
            seed: SEED,
        };
        DefaultInput::new(
            InputMethod::Default(Window::new("Handle"), PlatformInputKind::Focused),
            seeds,
        )
    }

    #[test]
    fn track_input_delay_tracked() {
        let sender = test_key_sender();

        // Force rng to generate delay > 0
        let result = sender.track_input_delay(KeyKind::Ctrl);
        assert_matches!(result, InputDelay::Tracked);
        assert!(sender.has_input_delay(KeyKind::Ctrl));
    }

    #[test]
    fn track_input_delay_already_tracked() {
        let sender = test_key_sender();
        sender
            .delay_map
            .borrow_mut()
            .insert(KeyKind::Ctrl, (3, false));

        let result = sender.track_input_delay(KeyKind::Ctrl);
        assert_matches!(result, InputDelay::AlreadyTracked);
    }

    #[test]
    fn update_input_delay_decrement_and_release_key() {
        let mut sender = test_key_sender();
        let count = 50;
        sender
            .delay_map
            .borrow_mut()
            .insert(KeyKind::Ctrl, (count, false));

        for _ in 0..count {
            sender.update(0);
        }
        // After `count` updates, key should be released and removed
        assert!(!sender.has_input_delay(KeyKind::Ctrl));
    }

    #[test]
    fn update_input_delay_refresh_mean_std_pair_every_interval() {
        let mut sender = test_key_sender();
        let original_pair = sender.delay_mean_std_pair;

        // Simulate tick before the interval: should NOT update
        sender.update(199);
        assert_eq!(sender.delay_mean_std_pair, original_pair);

        // Simulate tick AT the interval: should update
        sender.update(200);
        assert_ne!(sender.delay_mean_std_pair, original_pair);
    }
}
