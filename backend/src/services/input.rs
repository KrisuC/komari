use std::fmt::Debug;

use futures::StreamExt;
use platforms::Window;
use tokio::{
    spawn,
    sync::broadcast::{self, Receiver, Sender},
    task::JoinHandle,
};

use crate::{
    KeyBinding, OperationUpdate,
    bridge::{Input, InputMethod, InputReceiver, KeyKind},
    services::{Event, EventContext, EventHandler},
};

#[derive(Clone, Copy, Debug)]
pub enum InputEvent {
    KeyReceived(KeyKind),
}

impl Event for InputEvent {}

/// A service to handle input-related incoming requests.
pub trait InputService: Debug {
    fn subscribe_event(&self) -> Receiver<InputEvent>;

    fn subscribe_key(&self) -> Receiver<KeyBinding>;

    /// Updates `input` to use the new `window`.
    fn apply_window(&mut self, input: &mut dyn Input, window: Window);

    /// Updates `input` to use the new `method`.
    fn apply_method(&mut self, input: &mut dyn Input, method: InputMethod);
}

#[derive(Debug)]
pub struct DefaultInputService {
    input_tx: Sender<KeyBinding>,
    input_rx: Box<dyn InputReceiver>,
    event_tx: Sender<InputEvent>,
    event_task: Option<JoinHandle<()>>,
}

impl DefaultInputService {
    pub fn new(input_rx: impl InputReceiver) -> Self {
        Self {
            input_tx: broadcast::channel(1).0,
            input_rx: Box::new(input_rx),
            event_tx: broadcast::channel(5).0,
            event_task: None,
        }
    }

    fn run_task(&mut self) {
        if let Some(handle) = self.event_task.take() {
            handle.abort();
        }

        let input_tx = self.input_tx.clone();
        let event_tx = self.event_tx.clone();
        let mut input_stream = self.input_rx.as_stream();
        log::info!("[input_service] starting key listener task");
        let task = spawn(async move {
            log::info!("[input_service] key listener task started");
            let mut tick = 0u64;
            while let Some(key) = input_stream.next().await {
                tick += 1;
                log::info!("[input_service] key from stream: {key:?} (tick={tick})");
                let _ = event_tx.send(InputEvent::KeyReceived(key));
                let _ = input_tx.send(key.into());
            }
            log::warn!("[input_service] key listener task ended (stream closed)");
        });

        self.event_task = Some(task);
    }
}

impl InputService for DefaultInputService {
    fn subscribe_event(&self) -> Receiver<InputEvent> {
        self.event_tx.subscribe()
    }

    fn subscribe_key(&self) -> Receiver<KeyBinding> {
        self.input_tx.subscribe()
    }

    fn apply_window(&mut self, input: &mut dyn Input, window: Window) {
        input.set_window(window);
        self.input_rx.set_window(window);
        self.run_task();
    }

    fn apply_method(&mut self, input: &mut dyn Input, method: InputMethod) {
        input.set_method(method.clone());
        self.input_rx.set_method(method);
        self.run_task();
    }
}

pub struct InputEventHandler;

impl EventHandler<InputEvent> for InputEventHandler {
    fn handle(&mut self, context: &mut EventContext<'_>, event: InputEvent) {
        match event {
            InputEvent::KeyReceived(received_key) => {
                let toggle_actions_key = context.settings_service.settings().toggle_actions_key;
                log::info!(
                    "[input] key received: {:?}, toggle key: {:?} (enabled={})",
                    received_key,
                    toggle_actions_key.key,
                    toggle_actions_key.enabled,
                );

                if !toggle_actions_key.enabled {
                    return;
                }

                let key_binding: crate::models::KeyBinding = received_key.into();
                if toggle_actions_key.key == key_binding {
                    log::info!("[input] toggle hotkey matched!");
                    let update = if context.resources.operation.halting() {
                        OperationUpdate::Run
                    } else {
                        OperationUpdate::TemporaryHalt
                    };
                    log::info!(
                        "[input] toggle! halting={} update={update:?} state={:?}",
                        context.resources.operation.halting(),
                        context.resources.operation.state
                    );
                    context.operation_service.update(context.resources, update);
                    log::info!(
                        "[input] toggle done, new_state={:?}",
                        context.resources.operation.state
                    );
                }
            }
        }
    }
}
