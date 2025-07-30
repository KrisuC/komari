use std::{
    cell::{Ref, RefCell},
    rc::Rc,
};

#[cfg(test)]
use mockall::automock;
use mockall_double::double;
use platforms::{Window, capture::query_capture_name_window_pairs, input::InputKind};

#[double]
use crate::bridge::{Capture, InputReceiver};
use crate::{
    CaptureMode, InputMethod as DatabaseInputMethod, Settings,
    bridge::{Input, InputMethod},
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

#[cfg_attr(test, automock)]
impl SettingsService {
    /// Creates a new [`SettingsService`] from the provided `settings`.
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

    /// Gets the current [`Settings`] in use.
    pub fn current(&self) -> Ref<'_, Settings> {
        self.settings.borrow()
    }

    /// Gets a list of [`Window`] names to be used for selection.
    ///
    /// The index of a name corresponds to a [`Window`].
    pub fn current_window_names(&self) -> Vec<String> {
        self.capture_name_window_pairs
            .iter()
            .map(|(name, _)| name)
            .cloned()
            .collect::<Vec<_>>()
    }

    /// Gets the current selected [`Window`] index.
    pub fn current_selected_window_index(&self) -> Option<usize> {
        self.capture_selected_window_index
    }

    /// Gets the current selected [`Window`].
    ///
    /// If none is selected, the default [`Window`] is returned.
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

    /// Updates the list available of [`Window`]s from platform.
    pub fn update_windows(&mut self) {
        self.capture_name_window_pairs =
            query_capture_name_window_pairs().expect("supported platform");
    }

    /// Updates `input`, `input_receiver` and `capture` to use the [`Window`] specified by `index`.
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

    /// Updates the currently used [`Settings`] with `new_settings` and configures `operation`,
    /// `input`, `input_receiver` and `capture` to reflect the updated [`Settings`].
    pub fn update(
        &mut self,
        operation: &mut Operation,
        input: &mut dyn Input,
        input_receiver: &mut InputReceiver,
        capture: &mut Capture,
        new_settings: Settings,
    ) {
        *operation = operation.update_current(
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
    use std::sync::Mutex;

    use super::*;
    use crate::bridge::{InputMethod as BridgeInputMethod, MockInput};
    use crate::context::Operation;
    use crate::{CaptureMode, InputMethod};

    /// A mutex to guard against mocking static method from multiple threads.
    static MUTEX: Mutex<()> = Mutex::new(());

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
        let _guard = MUTEX.lock();
        let settings = Rc::new(RefCell::new(Settings {
            capture_mode: CaptureMode::WindowsGraphicsCapture,
            ..Default::default()
        }));
        let mut service = SettingsService::new(settings.clone());
        service.capture_name_window_pairs = vec![
            ("Foo".to_string(), Window::new("Foo")),
            ("Bar".to_string(), Window::new("Bar")),
        ];

        let mut mock_keys = MockInput::default();
        mock_keys.expect_set_method().withf(|method| match method {
            BridgeInputMethod::Rpc(_, _) => false,
            BridgeInputMethod::Default(window, kind) => {
                *window == Window::new("Bar") && matches!(kind, InputKind::Focused)
            }
        });

        let mut key_receiver = InputReceiver::default();
        let mut capture = Capture::default();
        capture
            .expect_set_window()
            .withf(|window| *window == Window::new("Bar"))
            .once();
        capture
            .expect_mode()
            .once()
            .return_const(CaptureMode::WindowsGraphicsCapture);
        capture
            .expect_set_mode()
            .withf(|mode| *mode == CaptureMode::WindowsGraphicsCapture)
            .once();

        let key_receiver_context = InputReceiver::new_context();
        key_receiver_context.expect().withf(|window, kind| {
            *window == Window::new("Bar") && matches!(kind, InputKind::Focused)
        });

        service.update_selected_window(&mut mock_keys, &mut key_receiver, &mut capture, Some(1));

        assert_eq!(service.current_selected_window_index(), Some(1));
        assert_eq!(service.current_window(), Window::new("Bar"));
    }

    #[test]
    fn update_settings_replaces_state_and_updates_components() {
        let _guard = MUTEX.lock();
        let settings = Rc::new(RefCell::new(Settings::default()));
        let mut service = SettingsService::new(settings.clone());
        let new_settings = Settings {
            input_method: InputMethod::Rpc,
            input_method_rpc_server_url: "http://localhost:9000".to_string(),
            cycle_run_stop: true,
            cycle_run_duration_millis: 1000,
            capture_mode: CaptureMode::WindowsGraphicsCapture,
            ..Default::default()
        };
        let mut mock_keys = MockInput::default();
        mock_keys.expect_set_method().withf(|method| match method {
            BridgeInputMethod::Rpc(window, url) => {
                *window == Window::new("MapleStoryClass") && url.as_str() == "http://localhost:9000"
            }
            BridgeInputMethod::Default(_, _) => false,
        });

        let mut key_receiver = InputReceiver::default();
        let key_receiver_context = InputReceiver::new_context();
        key_receiver_context.expect().withf(|window, kind| {
            *window == Window::new("MapleStoryClass") && matches!(kind, InputKind::Focused)
        });

        let mut capture = Capture::default();
        capture
            .expect_set_mode()
            .withf(|mode| *mode == CaptureMode::WindowsGraphicsCapture)
            .once();
        capture
            .expect_set_window()
            .withf(|window| *window == Window::new("MapleStoryClass"))
            .once();
        capture
            .expect_mode()
            .times(2)
            .return_const(CaptureMode::BitBlt);
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

    #[test]
    fn update_settings_input_receiver_foreground() {
        let _guard = MUTEX.lock();
        let settings = Rc::new(RefCell::new(Settings::default()));
        let mut service = SettingsService::new(settings.clone());
        let new_settings = Settings {
            capture_mode: CaptureMode::BitBltArea,
            ..Default::default()
        };
        let mut mock_keys = MockInput::default();
        mock_keys.expect_set_method().once();
        let mut key_receiver = InputReceiver::default();
        let key_receiver_context = InputReceiver::new_context();
        key_receiver_context.expect().withf(|window, kind| {
            *window == Window::new("MapleStoryClass") && matches!(kind, InputKind::Foreground)
        });

        let mut capture = Capture::default();
        capture
            .expect_window()
            .once()
            .returning(|| Window::new("MapleStoryClass"));
        capture
            .expect_mode()
            .times(2)
            .return_const(CaptureMode::BitBltArea);
        let mut op = Operation::Running;

        service.update(
            &mut op,
            &mut mock_keys,
            &mut key_receiver,
            &mut capture,
            new_settings.clone(),
        );
    }
}
