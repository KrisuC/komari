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
mod tests {
    use std::assert_matches::assert_matches;
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::bridge::{ImageCaptureKind, KeySenderMethod, MockKeySender};
    use crate::context::Operation;
    use crate::{CaptureMode, InputMethod};

    #[test]
    fn settings_service_initialization() {
        let settings = Rc::new(RefCell::new(Settings::default()));
        let service = SettingsService::new(settings.clone());

        assert_eq!(service.current_selected_handle_index(), None);
        assert_eq!(service.current().input_method, InputMethod::Default);
    }

    #[test]
    fn current_handle_fallbacks_to_default() {
        let settings = Rc::new(RefCell::new(Settings::default()));
        let service = SettingsService::new(settings.clone());

        // Without selected handle index
        let default = service.capture_default_handle;
        let current = service.current_handle();
        assert_eq!(current, default);
    }

    #[test]
    fn update_selected_handle_sets_index_and_updates() {
        let settings = Rc::new(RefCell::new(Settings {
            capture_mode: CaptureMode::WindowsGraphicsCapture,
            ..Default::default()
        }));
        let mut service = SettingsService::new(settings.clone());
        service.capture_handles = vec![
            ("Foo".to_string(), Handle::new("Foo")),
            ("Bar".to_string(), Handle::new("Bar")),
        ];

        let mut mock_keys = MockKeySender::default();
        mock_keys
            .expect_set_method()
            .withf(|method| match method {
                KeySenderMethod::Rpc(_, _) => false,
                KeySenderMethod::Default(handle, kind) => {
                    *handle == Handle::new("Bar") && matches!(kind, KeyInputKind::Fixed)
                }
            })
            .returning(|_| ());
        let mut key_receiver = KeyReceiver::new(service.current_handle(), KeyInputKind::Fixed);
        let mut capture = ImageCapture::new(service.current_handle(), CaptureMode::BitBlt);

        service.update_selected_handle(&mut mock_keys, &mut key_receiver, &mut capture, Some(1));

        assert_eq!(service.current_selected_handle_index(), Some(1));
        assert_eq!(service.current_handle(), Handle::new("Bar"));
        assert_matches!(capture.kind(), ImageCaptureKind::Wgc(_));
        // assert_matches!(key_receiver.kind(), KeyInputKind::Fixed);
        // assert_eq!(key_receiver.handle(), Handle::new("Bar"));
    }

    #[test]
    fn update_settings_replaces_state_and_updates_components() {
        let settings = Rc::new(RefCell::new(Settings::default()));
        let mut service = SettingsService::new(settings.clone());
        let new_settings = Settings {
            input_method: InputMethod::Rpc,
            input_method_rpc_server_url: "http://localhost:9000".to_string(),
            cycle_run_stop: true,
            cycle_run_duration_millis: 1000,
            ..Default::default()
        };
        let mut mock_keys = MockKeySender::default();
        let service_current_handle = service.current_handle();
        mock_keys
            .expect_set_method()
            .withf_st(move |method| match method {
                KeySenderMethod::Rpc(handle, url) => {
                    *handle == service_current_handle && url.as_str() == "http://localhost:9000"
                }
                KeySenderMethod::Default(_, _) => false,
            })
            .returning(|_| ());
        let mut key_receiver = KeyReceiver::new(service.current_handle(), KeyInputKind::Fixed);
        let mut capture = ImageCapture::new(service.current_handle(), CaptureMode::BitBlt);
        let mut op = Operation::Running;

        service.update(
            &mut op,
            &mut mock_keys,
            &mut key_receiver,
            &mut capture,
            new_settings.clone(),
        );

        let current = service.current();
        assert_matches!(op, Operation::RunUntil(_));
        assert_eq!(current.input_method, InputMethod::Rpc);
        assert_eq!(current.input_method_rpc_server_url, "http://localhost:9000");
    }

    // #[test]
    // fn update_handles_reloads_handle_list() {
    //     let settings = Rc::new(RefCell::new(Settings::default()));
    //     let mut service = SettingsService::new(settings.clone());

    //     let original_len = service.capture_handles.len();

    //     service.update_handles();
    //     let updated_len = service.capture_handles.len();

    //     // Cannot assume increase; just ensure function does not panic and changes may occur.
    //     assert!(updated_len >= 0);
    //     assert_eq!(updated_len, service.capture_handles.len());
    // }
}
