use std::{
    cell::{Ref, RefCell},
    rc::Rc,
};

use platforms::{Window, capture::query_capture_name_window_pairs, input::InputKind};

use crate::{
    CaptureMode, InputMethod as DatabaseInputMethod, Settings,
    bridge::{Capture, Input, InputMethod, InputReceiver},
    context::Operation,
};

/// A service to handle [`Settings`]-related incoming requests.
#[derive(Debug)]
pub struct SettingsService {
    settings: Rc<RefCell<Settings>>,
    capture_default_window: Window,
    capture_name_window_pairs: Vec<(String, Window)>,
    capture_selected_window_index: Option<usize>,
}

impl SettingsService {
    pub fn new(settings: Rc<RefCell<Settings>>) -> Self {
        // MapleStoryClass <- GMS
        // MapleStoryClassSG <- MSEA
        // MapleStoryClassTW <- TMS
        if cfg!(windows) {
            let window = Window::new("MapleStoryClass");

            return Self {
                settings,
                capture_default_window: window,
                capture_name_window_pairs: query_capture_name_window_pairs()
                    .expect("supported platform"),
                capture_selected_window_index: None,
            };
        }

        panic!("unsupported platform")
    }

    pub fn current(&self) -> Ref<'_, Settings> {
        self.settings.borrow()
    }

    pub fn current_window_names(&self) -> Vec<String> {
        self.capture_name_window_pairs
            .iter()
            .map(|(name, _)| name)
            .cloned()
            .collect::<Vec<_>>()
    }

    pub fn current_selected_window_index(&self) -> Option<usize> {
        self.capture_selected_window_index
    }

    pub fn current_window(&self) -> Window {
        self.capture_selected_window_index
            .and_then(|index| {
                self.capture_name_window_pairs
                    .get(index)
                    .map(|(_, handle)| handle)
                    .copied()
            })
            .unwrap_or(self.capture_default_window)
    }

    pub fn update_windows(&mut self) {
        self.capture_name_window_pairs =
            query_capture_name_window_pairs().expect("supported platform");
    }

    pub fn update_selected_window(
        &mut self,
        input: &mut dyn Input,
        input_receiver: &mut InputReceiver,
        capture: &mut Capture,
        index: Option<usize>,
    ) {
        self.capture_selected_window_index = index;
        self.update_capture(capture, true);
        self.update_inputs(input, input_receiver, capture);
    }

    /// Updates the currently used [`Settings`] from `new_settings` and configures `keys`,
    /// `key_receiver` and `capture`.
    pub fn update(
        &mut self,
        operation: &mut Operation,
        input: &mut dyn Input,
        input_receiver: &mut InputReceiver,
        capture: &mut Capture,
        new_settings: Settings,
    ) {
        operation.update_current(
            new_settings.cycle_run_stop,
            new_settings.cycle_run_duration_millis,
            new_settings.cycle_stop_duration_millis,
        );
        *self.settings.borrow_mut() = new_settings;
        self.update_capture(capture, false);
        self.update_inputs(input, input_receiver, capture);
    }

    fn update_capture(&self, capture: &mut Capture, forced: bool) {
        let settings = self.current();
        if forced || capture.mode() != settings.capture_mode {
            capture.set_mode(settings.capture_mode);
            capture.set_window(self.current_window());
        }
    }

    fn update_inputs(
        &self,
        input: &mut dyn Input,
        input_receiver: &mut InputReceiver,
        capture: &Capture,
    ) {
        let settings = self.current();
        let (window, kind) = if matches!(capture.mode(), CaptureMode::BitBltArea) {
            (capture.window(), InputKind::Foreground)
        } else {
            (self.current_window(), InputKind::Focused)
        };

        *input_receiver = InputReceiver::new(window, kind);
        match settings.input_method {
            DatabaseInputMethod::Default => {
                input.set_method(InputMethod::Default(window, kind));
            }
            DatabaseInputMethod::Rpc => {
                input.set_method(InputMethod::Rpc(
                    window,
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

        assert_eq!(service.current_selected_window_index(), None);
        assert_eq!(service.current().input_method, InputMethod::Default);
    }

    #[test]
    fn current_handle_fallbacks_to_default() {
        let settings = Rc::new(RefCell::new(Settings::default()));
        let service = SettingsService::new(settings.clone());

        // Without selected handle index
        let default = service.capture_default_window;
        let current = service.current_window();
        assert_eq!(current, default);
    }

    #[test]
    fn update_selected_handle_sets_index_and_updates() {
        let settings = Rc::new(RefCell::new(Settings {
            capture_mode: CaptureMode::WindowsGraphicsCapture,
            ..Default::default()
        }));
        let mut service = SettingsService::new(settings.clone());
        service.capture_name_window_pairs = vec![
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
        let mut key_receiver = KeyReceiver::new(service.current_window(), KeyInputKind::Fixed);
        let mut capture = ImageCapture::new(service.current_window(), CaptureMode::BitBlt);

        service.update_selected_window(&mut mock_keys, &mut key_receiver, &mut capture, Some(1));

        assert_eq!(service.current_selected_window_index(), Some(1));
        assert_eq!(service.current_window(), Handle::new("Bar"));
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
        let service_current_handle = service.current_window();
        mock_keys
            .expect_set_method()
            .withf_st(move |method| match method {
                KeySenderMethod::Rpc(handle, url) => {
                    *handle == service_current_handle && url.as_str() == "http://localhost:9000"
                }
                KeySenderMethod::Default(_, _) => false,
            })
            .returning(|_| ());
        let mut key_receiver = KeyReceiver::new(service.current_window(), KeyInputKind::Fixed);
        let mut capture = ImageCapture::new(service.current_window(), CaptureMode::BitBlt);
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
