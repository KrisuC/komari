use std::{
    cell::RefCell,
    env,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use dyn_clone::clone_box;
#[cfg(debug_assertions)]
use log::debug;
use opencv::{
    core::{Vector, VectorToVec},
    imgcodecs::imencode_def,
};
use platforms::windows::{self, Handle, KeyInputKind, KeyReceiver};
use strum::IntoEnumIterator;
use tokio::sync::broadcast;

use crate::{
    Action,
    bridge::{DefaultKeySender, ImageCapture, ImageCaptureKind, KeySender, KeySenderMethod},
    buff::{Buff, BuffKind, BuffState},
    database::{CaptureMode, InputMethod, KeyBinding, query_seeds, query_settings},
    database_event_receiver,
    detect::{CachedDetector, Detector},
    mat::OwnedMat,
    minimap::{Minimap, MinimapState},
    navigation::Navigator,
    network::{DiscordNotification, NotificationKind},
    player::{PanicTo, Panicking, Player, PlayerState},
    request_handler::DefaultRequestHandler,
    rng::Rng,
    rotator::Rotator,
    skill::{Skill, SkillKind, SkillState},
};
#[cfg(test)]
use crate::{Settings, bridge::MockKeySender, detect::MockDetector};

const FPS: u32 = 30;
const PENDING_HALT_SECS: u64 = 12;
pub const MS_PER_TICK: u64 = MS_PER_TICK_F32 as u64;
pub const MS_PER_TICK_F32: f32 = 1000.0 / FPS as f32;

/// A control flow to use after a contextual state update.
#[derive(Debug)]
pub enum ControlFlow<T> {
    /// The contextual state is updated immediately.
    Immediate(T),
    /// The contextual state is updated in the next tick.
    Next(T),
}

/// Represents a contextual state.
pub trait Contextual {
    /// The inner state that is persistent through each [`Contextual::update`] tick.
    type Persistent = ();

    /// Updates the contextual state.
    ///
    /// This is basically a state machine.
    ///
    /// Updating is performed on each tick and the behavior whether to continue
    /// updating in the same tick or next is decided by [`ControlFlow`]. The state
    /// can transition or stay the same.
    fn update(self, context: &Context, persistent: &mut Self::Persistent) -> ControlFlow<Self>
    where
        Self: Sized;
}

/// A struct that stores the game information.
#[derive(Debug)]
pub struct Context {
    /// The `MapleStory` class game handle.
    ///
    /// This is always the default game handle (e.g. MapleStoryClass).
    pub handle: Handle,
    /// A struct to send key inputs.
    pub keys: Box<dyn KeySender>,
    pub rng: Rng,
    /// A struct for sending notifications through web hook.
    pub notification: DiscordNotification,
    /// A struct to detect game information.
    ///
    /// This is [`None`] when no frame as ever been captured.
    pub detector: Option<Box<dyn Detector>>,
    /// The minimap contextual state.
    pub minimap: Minimap,
    /// The player contextual state.
    pub player: Player,
    /// The skill contextual states.
    pub skills: [Skill; SkillKind::COUNT],
    /// The buff contextual states.
    pub buffs: [Buff; BuffKind::COUNT],
    /// The bot current's operation.
    pub operation: Operation,
    /// The game current tick.
    ///
    /// This is increased on each update tick.
    pub tick: u64,
    /// Whether minimap changed to detecting on the current tick.
    pub did_minimap_changed: bool,
}

impl Context {
    #[cfg(test)]
    pub fn new(keys: Option<MockKeySender>, detector: Option<MockDetector>) -> Self {
        Context {
            handle: Handle::new(""),
            keys: Box::new(keys.unwrap_or_default()),
            rng: Rng::new(rand::random()),
            notification: DiscordNotification::new(Rc::new(RefCell::new(Settings::default()))),
            detector: detector.map(|detector| Box::new(detector) as Box<dyn Detector>),
            minimap: Minimap::Detecting,
            player: Player::Detecting,
            skills: [Skill::Detecting; SkillKind::COUNT],
            buffs: [Buff::No; BuffKind::COUNT],
            operation: Operation::Running,
            tick: 0,
            did_minimap_changed: false,
        }
    }

    #[inline]
    pub fn detector_unwrap(&self) -> &dyn Detector {
        self.detector
            .as_ref()
            .expect("detector is not available because no frame has ever been captured")
            .as_ref()
    }

    #[inline]
    pub fn detector_cloned_unwrap(&self) -> Box<dyn Detector> {
        clone_box(self.detector_unwrap())
    }
}

#[derive(Debug)]
pub enum Operation {
    HaltUntil(Instant),
    Halting,
    Running,
    RunUntil(Instant),
}

impl Operation {
    #[inline]
    pub fn halting(&self) -> bool {
        matches!(self, Operation::Halting | Operation::HaltUntil(_))
    }
}

pub fn init() {
    static LOOPING: AtomicBool = AtomicBool::new(false);

    if LOOPING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::Acquire)
        .is_ok()
    {
        let dll = env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("onnxruntime.dll");

        ort::init_from(dll.to_str().unwrap()).commit().unwrap();
        windows::init();
        thread::spawn(|| {
            let tokio_rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            let _tokio_guard = tokio_rt.enter();
            tokio_rt.block_on(async {
                update_loop();
            });
        });
    }
}

#[inline]
fn update_loop() {
    // MapleStoryClass <- GMS
    // MapleStoryClassSG <- MSEA
    // MapleStoryClassTW <- TMS
    let handle = Handle::new("MapleStoryClass");
    let mut rotator = Rotator::default();
    let mut navigator = Navigator::default();
    let mut actions = Vec::<Action>::new();
    let mut minimap = None; // Override by UI
    let mut minimap_preset = None; // Override by UI
    let mut character = None; // Override by UI
    let mut buffs = vec![];
    let settings = query_settings();
    let seeds = query_seeds(); // Fixed, unchanged
    let rng = Rng::new(seeds.seed); // Create one for Context

    let key_sender_method = if let InputMethod::Rpc = settings.input_method {
        KeySenderMethod::Rpc(handle, settings.input_method_rpc_server_url.clone())
    } else {
        match settings.capture_mode {
            CaptureMode::BitBlt | CaptureMode::WindowsGraphicsCapture => {
                KeySenderMethod::Default(handle, KeyInputKind::Fixed)
            }
            // This shouldn't matter because we have to get the Handle from the box capture anyway
            CaptureMode::BitBltArea => KeySenderMethod::Default(handle, KeyInputKind::Foreground),
        }
    };
    let mut keys = DefaultKeySender::new(key_sender_method, seeds);
    let key_sender = broadcast::channel::<KeyBinding>(1).0; // Callback to UI
    let mut key_receiver = KeyReceiver::new(handle, KeyInputKind::Fixed);

    let mut capture_handles = Vec::<(String, Handle)>::new();
    let mut selected_capture_handle = None;
    let mut image_capture = ImageCapture::new(handle, settings.capture_mode);
    if let ImageCaptureKind::BitBltArea(capture) = image_capture.kind() {
        key_receiver = KeyReceiver::new(capture.handle(), KeyInputKind::Foreground);
        keys.set_method(KeySenderMethod::Default(
            capture.handle(),
            KeyInputKind::Foreground,
        ));
    }

    let settings = Rc::new(RefCell::new(settings));
    let mut context = Context {
        handle,
        keys: Box::new(keys),
        rng,
        notification: DiscordNotification::new(settings.clone()),
        detector: None,
        minimap: Minimap::Detecting,
        player: Player::Idle,
        skills: [Skill::Detecting],
        buffs: [Buff::No; BuffKind::COUNT],
        operation: Operation::Halting,
        tick: 0,
        did_minimap_changed: false,
    };
    let mut player_state = PlayerState::default();
    let mut minimap_state = MinimapState::default();
    let mut skill_states = SkillKind::iter()
        .map(SkillState::new)
        .collect::<Vec<SkillState>>();
    let mut buff_states = BuffKind::iter()
        .map(BuffState::new)
        .collect::<Vec<BuffState>>();
    // When minimap changes, a pending halt will be queued. This helps ensure that if any
    // accidental or intended (e.g. navigating) minimap change occurs, it will try to wait for a
    // specified threshold to pass before determining panicking is needed. This can be beneficial
    // when navigator falsely navigates to a wrong unknown location.
    let mut pending_halt = None;
    let mut database_event_receiver = database_event_receiver();

    #[cfg(debug_assertions)]
    let mut recording_images_id = None;
    #[cfg(debug_assertions)]
    let mut infering_rune = None;

    loop_with_fps(FPS, || {
        let mat = image_capture.grab().map(OwnedMat::new_from_frame);
        let was_player_alive = !player_state.is_dead();
        let was_player_navigating = navigator.was_last_point_available_or_completed();
        let mut was_cycled_to_stop = false;
        let detector = mat.map(CachedDetector::new);

        context.tick += 1;
        context.operation = match context.operation {
            // Imply run/stop cycle enabled
            Operation::HaltUntil(instant) => {
                let now = Instant::now();
                if now < instant {
                    Operation::HaltUntil(instant)
                } else {
                    Operation::RunUntil(
                        now + Duration::from_millis(settings.borrow().cycle_run_duration_millis),
                    )
                }
            }
            Operation::Halting => Operation::Halting,
            Operation::Running => Operation::Running,
            // Imply run/stop cycle enabled
            Operation::RunUntil(instant) => {
                let now = Instant::now();
                if now < instant {
                    Operation::RunUntil(instant)
                } else {
                    was_cycled_to_stop = true;
                    Operation::HaltUntil(
                        now + Duration::from_millis(settings.borrow().cycle_stop_duration_millis),
                    )
                }
            }
        };
        if let Some(detector) = detector {
            let was_minimap_idle = matches!(context.minimap, Minimap::Idle(_));

            context.detector = Some(Box::new(detector));
            context.minimap = fold_context(&context, context.minimap, &mut minimap_state);
            context.did_minimap_changed =
                was_minimap_idle && matches!(context.minimap, Minimap::Detecting);
            context.player = fold_context(&context, context.player, &mut player_state);
            for (i, state) in skill_states
                .iter_mut()
                .enumerate()
                .take(context.skills.len())
            {
                context.skills[i] = fold_context(&context, context.skills[i], state);
            }
            for (i, state) in buff_states.iter_mut().enumerate().take(context.buffs.len()) {
                context.buffs[i] = fold_context(&context, context.buffs[i], state);
            }

            // This must always be done last
            navigator.update(&context);
            if navigator.navigate_player(&context, &mut player_state) {
                rotator.rotate_action(&context, &mut player_state);
            }
        }
        // TODO: Maybe should not downcast but really don't want to public update_input_delay
        // method
        context
            .keys
            .as_any_mut()
            .downcast_mut::<DefaultKeySender>()
            .unwrap()
            .update_input_delay(context.tick);
        context.notification.update_scheduled_frames(|| {
            to_png(context.detector.as_ref().map(|detector| detector.mat()))
        });

        // Poll requests, keys and update scheduled notifications frames
        let mut settings_borrow_mut = settings.borrow_mut();
        // I know what you are thinking...
        let mut handler = DefaultRequestHandler {
            context: &mut context,
            character: &mut character,
            settings: &mut settings_borrow_mut,
            buffs: &mut buffs,
            buff_states: &mut buff_states,
            actions: &mut actions,
            rotator: &mut rotator,
            navigator: &mut navigator,
            player: &mut player_state,
            minimap: &mut minimap_state,
            minimap_data: &mut minimap,
            minimap_data_preset: &mut minimap_preset,
            key_sender: &key_sender,
            key_receiver: &mut key_receiver,
            image_capture: &mut image_capture,
            capture_handles: &mut capture_handles,
            selected_capture_handle: &mut selected_capture_handle,
            database_event_receiver: &mut database_event_receiver,
            #[cfg(debug_assertions)]
            recording_images_id: &mut recording_images_id,
            #[cfg(debug_assertions)]
            infering_rune: &mut infering_rune,
        };
        handler.poll_request();

        // Go to town on stop cycle
        if was_cycled_to_stop {
            handler.rotator.reset_queue();
            handler.player.clear_actions_aborted(false);
            handler.context.player = Player::Panicking(Panicking::new(PanicTo::Town));
        }
        // Upon accidental or white roomed causing map to change,
        // abort actions and send notification
        if handler.minimap_data.is_some() && !handler.context.operation.halting() {
            if was_player_navigating {
                pending_halt = None;
            }

            let player_died = was_player_alive && handler.player.is_dead();
            let player_panicking = matches!(
                handler.context.player,
                Player::Panicking(Panicking {
                    to: PanicTo::Channel,
                    ..
                })
            );
            let pending_halt_reached = pending_halt.is_some_and(|instant| {
                Instant::now().duration_since(instant).as_secs() >= PENDING_HALT_SECS
            });
            let can_halt_or_notify =
                pending_halt_reached || (handler.context.did_minimap_changed && !player_panicking);
            match (
                player_died,
                can_halt_or_notify,
                handler.settings.stop_on_fail_or_change_map,
            ) {
                (true, _, _) => {
                    handler.update_context_halting(true, true);
                }
                (_, true, true) => {
                    if pending_halt.is_none() {
                        pending_halt = Some(Instant::now());
                    } else {
                        pending_halt = None;
                        handler.update_context_halting(true, false);
                        handler.context.player = Player::Panicking(Panicking::new(PanicTo::Town));
                    }
                }
                _ => (),
            }
            if can_halt_or_notify && pending_halt.is_none() {
                drop(settings_borrow_mut); // For notification to borrow immutably
                let _ = context
                    .notification
                    .schedule_notification(NotificationKind::FailOrMapChange);
            }
        }
    });
}

#[inline]
fn fold_context<C>(
    context: &Context,
    contextual: C,
    persistent: &mut <C as Contextual>::Persistent,
) -> C
where
    C: Contextual,
{
    let mut control_flow = contextual.update(context, persistent);
    loop {
        match control_flow {
            ControlFlow::Immediate(contextual) => {
                control_flow = contextual.update(context, persistent);
            }
            ControlFlow::Next(contextual) => return contextual,
        }
    }
}

#[inline]
fn loop_with_fps(fps: u32, mut on_tick: impl FnMut()) {
    #[cfg(debug_assertions)]
    const LOG_INTERVAL_SECS: u64 = 5;

    let nanos_per_frame = (1_000_000_000 / fps) as u128;
    #[cfg(debug_assertions)]
    let mut last_logged_instant = Instant::now();

    loop {
        let start = Instant::now();

        on_tick();

        let now = Instant::now();
        let elapsed_duration = now.duration_since(start);
        let elapsed_nanos = elapsed_duration.as_nanos();
        if elapsed_nanos <= nanos_per_frame {
            thread::sleep(Duration::new(0, (nanos_per_frame - elapsed_nanos) as u32));
        } else {
            #[cfg(debug_assertions)]
            if now.duration_since(last_logged_instant).as_secs() >= LOG_INTERVAL_SECS {
                last_logged_instant = now;
                debug!(target: "context", "ticking running late at {}ms", elapsed_duration.as_millis());
            }
        }
    }
}

#[inline]
fn to_png(frame: Option<&OwnedMat>) -> Option<Vec<u8>> {
    frame.and_then(|image| {
        let mut bytes = Vector::new();
        imencode_def(".png", image, &mut bytes).ok()?;
        Some(bytes.to_vec())
    })
}
