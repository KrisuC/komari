use crate::{Error, Result, Window};
#[cfg(windows)]
use crate::{windows::WindowsInput, windows::WindowsInputReceiver};

#[derive(Debug, Clone, Copy)]
pub enum MouseKind {
    Move,
    Click,
    Scroll,
}

#[derive(Debug, Clone, Copy)]
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

#[derive(Debug, Clone, Copy)]
pub enum InputKind {
    /// Sends input only if the foreground window is [`Window`] and focused.
    Focused,
    /// Sends input only if the foreground window is not [`Window`], overlaps with [`Window`] and
    /// is focused.
    Foreground,
}

/// Struct for sending key and mouse inputs.
#[derive(Debug)]
pub struct Input {
    #[cfg(windows)]
    windows: WindowsInput,
}

impl Input {
    pub fn new(window: Window, kind: InputKind) -> Result<Self> {
        if cfg!(windows) {
            return Ok(Self {
                windows: WindowsInput::new(window.windows, kind),
            });
        }

        Err(Error::PlatformNotSupported)
    }

    /// Sends mouse `kind` with coordinates `x`, `y` in relative to the provided [`Window`].
    pub fn send_mouse(&self, x: i32, y: i32, kind: MouseKind) -> Result<()> {
        if cfg!(windows) {
            return self.windows.send_mouse(x, y, kind);
        }

        Err(Error::PlatformNotSupported)
    }

    /// Sends a single key press `kind`.
    pub fn send_key(&self, kind: KeyKind) -> Result<()> {
        if cfg!(windows) {
            return self.windows.send_key(kind);
        }

        Err(Error::PlatformNotSupported)
    }

    /// Holds down key `kind`.
    pub fn send_key_down(&self, kind: KeyKind) -> Result<()> {
        if cfg!(windows) {
            return self.windows.send_key_down(kind);
        }

        Ok(())
    }

    /// Releases key `kind`.
    pub fn send_key_up(&self, kind: KeyKind) -> Result<()> {
        if cfg!(windows) {
            return self.windows.send_key_up(kind);
        }

        Err(Error::PlatformNotSupported)
    }
}

#[derive(Debug)]
pub struct InputReceiver {
    #[cfg(windows)]
    windows: WindowsInputReceiver,
}

impl InputReceiver {
    pub fn new(window: Window, input_kind: InputKind) -> Result<Self> {
        if cfg!(windows) {
            return Ok(Self {
                windows: WindowsInputReceiver::new(window.windows, input_kind),
            });
        }

        Err(Error::PlatformNotSupported)
    }

    /// Attempts to receive a key stroke previously sent from the OS.
    pub fn try_recv(&mut self) -> Result<KeyKind> {
        if cfg!(windows) {
            return self.windows.try_recv().ok_or(Error::KeyNotReceived);
        }

        Err(Error::PlatformNotSupported)
    }
}
