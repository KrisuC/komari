#[cfg(windows)]
use crate::windows::{WgcCapture, WindowBoxCapture, WindowsCapture};
use crate::{Error, Result, Window, windows::BitBltCapture};

#[derive(Debug, Clone)]
pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
    // TODO: Color format? Currently always BGRA
}

#[cfg(windows)]
#[derive(Debug)]
pub enum WindowsCaptureKind {
    BitBlt,
    BitBltArea,
    Wgc(u64),
}

#[derive(Debug)]
pub struct Capture {
    window: Window,
    #[cfg(windows)]
    windows: WindowsCapture,
}

impl Capture {
    pub fn new(window: Window) -> Result<Self> {
        if cfg!(windows) {
            return Ok(Self {
                window,
                windows: WindowsCapture::BitBlt(BitBltCapture::new(window.windows, false)),
            });
        }

        Err(Error::PlatformNotSupported)
    }

    pub fn grab(&mut self) -> Result<Frame> {
        if cfg!(windows) {
            return self.windows.grab();
        }

        Err(Error::PlatformNotSupported)
    }

    #[cfg(windows)]
    pub fn set_capture_kind(&mut self, kind: WindowsCaptureKind) -> Result<()> {
        self.windows = match kind {
            WindowsCaptureKind::BitBlt => {
                WindowsCapture::BitBlt(BitBltCapture::new(self.window.windows, false))
            }
            WindowsCaptureKind::BitBltArea => {
                WindowsCapture::BitBltArea(WindowBoxCapture::default())
            }
            WindowsCaptureKind::Wgc(frame_timeout_millis) => {
                WindowsCapture::Wgc(WgcCapture::new(self.window.windows, frame_timeout_millis)?)
            }
        };

        Ok(())
    }
}
