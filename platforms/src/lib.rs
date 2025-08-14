#![feature(str_from_raw_parts)]

use thiserror::Error;

#[cfg(windows)]
use crate::windows::{Handle, HandleKind, client_to_monitor_or_frame};

pub mod capture;
pub mod input;

#[cfg(windows)]
mod windows;

pub type Result<T> = core::result::Result<T, Error>;

/// Platform-specific error.
#[derive(Error, PartialEq, Clone, Debug)]
pub enum Error {
    #[error("key was not sent due to the window not focused or other error")]
    KeyNotSent,
    #[error("key not found")]
    KeyNotFound,
    #[error("key not received because there is no key event")]
    KeyNotReceived,
    #[error("key state currently cannot be retrieved")]
    KeyStateNotAvailable,
    #[error("mouse was not sent due to the window not focused or other error")]
    MouseNotSent,

    #[error("window not found")]
    WindowNotFound,
    #[error("the current window size is invalid")]
    WindowInvalidSize,
    #[error("window capture frame is not available")]
    WindowFrameNotAvailable,

    #[error("platform is not supported")]
    PlatformNotSupported,

    #[cfg(windows)]
    #[error("win32 API error {0}: {1}")]
    Win32(u32, String),
}

/// Relativeness of a point to be converted to.
#[derive(Debug)]
pub enum CoordinateRelative {
    /// Point will be converted with x, y coordinates relative to the monitor of the specified `[Window]`.
    Monitor,
    /// Point will be converted with x, y coordinates relative to the client area of the specified `[Window]`.
    Window,
}

/// Represents converted coordinates as specified by [`CoordinateRelative`].
#[derive(Debug)]
pub struct ConvertedCoordinates {
    /// The width of the monitor or client area.
    pub width: i32,
    /// The height of the monitor or client area.
    pub height: i32,
    /// x coordinate in relative to the monitor or client area.
    pub x: i32,
    /// y coordinate in relative to the monitor or client area.
    pub y: i32,
}

/// A platform-specific handle to a window on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    #[cfg(windows)]
    windows: Handle,
}

impl Window {
    #[cfg(windows)]
    pub fn new(class: &'static str) -> Self {
        Self {
            windows: Handle::new(HandleKind::Dynamic(class)),
        }
    }

    #[inline]
    pub fn convert_coordinate(
        &self,
        x: i32,
        y: i32,
        relative: CoordinateRelative,
    ) -> Result<ConvertedCoordinates> {
        if cfg!(windows) {
            return client_to_monitor_or_frame(
                self.windows,
                x,
                y,
                matches!(relative, CoordinateRelative::Monitor),
            );
        }

        Err(Error::PlatformNotSupported)
    }
}

#[cfg(windows)]
impl From<Handle> for Window {
    fn from(value: Handle) -> Self {
        Self { windows: value }
    }
}

pub fn init() {
    if cfg!(windows) {
        windows::init();
    }
}
