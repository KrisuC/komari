use std::{
    cell::{Ref, RefCell},
    rc::Rc,
};

use platforms::windows::{Handle, KeyInputKind, KeyReceiver, query_capture_handles};

use crate::{
    CaptureMode, InputMethod, Settings,
    bridge::{ImageCapture, ImageCaptureKind, KeySender, KeySenderMethod},
    context::Operation,
};

/// A service to handle [`Settings`]-related incoming requests.
#[derive(Debug)]
pub struct SettingsService {
    settings: Rc<RefCell<Settings>>,
    capture_default_handle: Handle,
    capture_handles: Vec<(String, Handle)>,
    capture_selected_handle_index: Option<usize>,
}

impl SettingsService {
    pub fn new(settings: Rc<RefCell<Settings>>) -> Self {
        // MapleStoryClass <- GMS
        // MapleStoryClassSG <- MSEA
        // MapleStoryClassTW <- TMS
        let handle = Handle::new("MapleStoryClass");

        Self {
            settings,
            capture_default_handle: handle,
            capture_handles: query_capture_handles(),
            capture_selected_handle_index: None,
        }
    }

    pub fn current(&self) -> Ref<'_, Settings> {
        self.settings.borrow()
    }

    pub fn current_handle_names(&self) -> Vec<String> {
        self.capture_handles
            .iter()
            .map(|(name, _)| name)
            .cloned()
            .collect::<Vec<_>>()
    }

    pub fn current_selected_handle_index(&self) -> Option<usize> {
        self.capture_selected_handle_index
    }

    pub fn current_handle(&self) -> Handle {
        self.capture_selected_handle_index
            .and_then(|index| {
                self.capture_handles
                    .get(index)
                    .map(|(_, handle)| handle)
                    .copied()
            })
            .unwrap_or(self.capture_default_handle)
    }

    pub fn update_handles(&mut self) {
        self.capture_handles = query_capture_handles();
    }

    pub fn update_selected_handle(
        &mut self,
        keys: &mut dyn KeySender,
        key_receiver: &mut KeyReceiver,
        capture: &mut ImageCapture,
        index: Option<usize>,
    ) {
        self.capture_selected_handle_index = index;
        self.update_capture(capture, true);
        self.update_keys(keys, key_receiver, capture.kind());
    }

    /// Updates the currently used [`Settings`] from `new_settings` and configures `keys`,
    /// `key_receiver` and `capture`.
    pub fn update(
        &mut self,
        operation: &mut Operation,
        keys: &mut dyn KeySender,
        key_receiver: &mut KeyReceiver,
        capture: &mut ImageCapture,
        new_settings: Settings,
    ) {
        operation.update_current(
            new_settings.cycle_run_stop,
            new_settings.cycle_run_duration_millis,
            new_settings.cycle_stop_duration_millis,
        );
        *self.settings.borrow_mut() = new_settings;
        self.update_capture(capture, false);
        self.update_keys(keys, key_receiver, capture.kind());
    }

    fn update_capture(&self, capture: &mut ImageCapture, forced: bool) {
        let settings = self.current();
        let current_mode = match capture.kind() {
            ImageCaptureKind::BitBlt(_) => CaptureMode::BitBlt,
            ImageCaptureKind::Wgc(_) => CaptureMode::WindowsGraphicsCapture,
            ImageCaptureKind::BitBltArea(_) => CaptureMode::BitBltArea,
        };
        if forced || current_mode != settings.capture_mode {
            capture.set_mode(self.current_handle(), settings.capture_mode);
        }
    }

    fn update_keys(
        &self,
        keys: &mut dyn KeySender,
        key_receiver: &mut KeyReceiver,
        capture_kind: &ImageCaptureKind,
    ) {
        let settings = self.current();
        let (handle, kind) = if let ImageCaptureKind::BitBltArea(capture) = capture_kind {
            (capture.handle(), KeyInputKind::Foreground)
        } else {
            (self.current_handle(), KeyInputKind::Fixed)
        };

        *key_receiver = KeyReceiver::new(handle, kind);
        match settings.input_method {
            InputMethod::Default => {
                keys.set_method(KeySenderMethod::Default(handle, kind));
            }
            InputMethod::Rpc => {
                keys.set_method(KeySenderMethod::Rpc(
                    handle,
                    settings.input_method_rpc_server_url.clone(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {}
