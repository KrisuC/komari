use std::{
    sync::{
        Arc, Barrier, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM},
        Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        UI::WindowsAndMessaging::{
            DispatchMessageW, EnumWindows, GetMessageW, GWL_EXSTYLE, GWL_STYLE,
            GetWindowLongPtrW, IsWindowVisible, MSG, TranslateMessage, WS_DISABLED,
            WS_EX_TOOLWINDOW,
        },
    },
    core::{BOOL, Owned},
};

mod bitblt;
mod handle;
mod input;
mod wgc;
mod window_box;

pub use {bitblt::*, handle::*, input::*, wgc::*, window_box::*};

use crate::{Error, Result, capture::Frame};

#[derive(Debug)]
pub enum WindowsCapture {
    BitBlt(BitBltCapture),
    BitBltArea(WindowBoxCapture),
    Wgc(WgcCapture),
}

impl WindowsCapture {
    #[inline]
    pub fn grab(&mut self) -> Result<Frame> {
        match self {
            WindowsCapture::BitBlt(capture) => capture.grab(),
            WindowsCapture::BitBltArea(capture) => capture.grab(),
            WindowsCapture::Wgc(capture) => capture.grab(),
        }
    }
}

pub fn init() {
    static INITIALIZED: AtomicBool = AtomicBool::new(false);

    if INITIALIZED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::Acquire)
        .is_ok()
    {
        // Install on the main thread — Dioxus/tao provides the message pump.
        input::init();
    }
}

impl Error {
    #[inline]
    pub(crate) fn from_last_win_error() -> Error {
        Error::from(windows::core::Error::from_win32())
    }
}

impl From<windows::core::Error> for Error {
    fn from(error: windows::core::Error) -> Self {
        Error::Win32(error.code().0 as u32, error.message())
    }
}

pub fn query_capture_handles() -> Vec<Handle> {
    unsafe extern "system" fn callback(handle: HWND, params: LPARAM) -> BOOL {
        if !unsafe { IsWindowVisible(handle) }.as_bool() {
            return true.into();
        }

        let mut cloaked = 0u32;
        let _ = unsafe {
            DwmGetWindowAttribute(
                handle,
                DWMWA_CLOAKED,
                (&raw mut cloaked).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        };
        if cloaked != 0 {
            return true.into();
        }

        let style = unsafe { GetWindowLongPtrW(handle, GWL_STYLE) } as u32;
        let ex_style = unsafe { GetWindowLongPtrW(handle, GWL_EXSTYLE) } as u32;
        if style & WS_DISABLED.0 != 0 || ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return true.into();
        }

        let vec_ptr = params.0 as *mut Vec<Handle>;
        let vec = unsafe { &mut *vec_ptr };
        vec.push(Handle::new(HandleKind::Fixed(handle.into())));

        true.into()
    }

    let mut vec = Vec::new();
    let _ = unsafe { EnumWindows(Some(callback), LPARAM(&raw mut vec as isize)) };
    vec
}
