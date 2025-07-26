use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
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
        let barrier = Arc::new(Barrier::new(2));
        let keys_barrier = barrier.clone();
        thread::spawn(move || {
            let _hook = input::init();
            let mut msg = MSG::default();
            keys_barrier.wait();
            while unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.as_bool() {
                unsafe {
                    let _ = TranslateMessage(&raw const msg);
                    let _ = DispatchMessageW(&raw const msg);
                }
            }
        });
        barrier.wait();
    }
}

impl Error {
    #[inline]
    pub fn from_last_win_error() -> Error {
        Error::from(windows::core::Error::from_win32())
    }
}

impl From<windows::core::Error> for Error {
    fn from(error: windows::core::Error) -> Self {
        Error::Win32(error.code().0 as u32, error.message())
    }
}
