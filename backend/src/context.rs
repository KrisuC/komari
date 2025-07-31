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
use mockall_double::double;
use opencv::{
    core::{Vector, VectorToVec},
    imgcodecs::imencode_def,
};
use platforms::Error;
use strum::IntoEnumIterator;

use crate::{
    CycleRunStopMode,
    bridge::Input,
    buff::{Buff, BuffKind, BuffState},
    database::{query_seeds, query_settings},
    detect::{CachedDetector, Detector},
    mat::OwnedMat,
    minimap::{Minimap, MinimapState},
    network::{DiscordNotification, NotificationKind},
    player::{PanicTo, Panicking, Player, PlayerState},
    rng::Rng,
    services::{DefaultService, PollArgs},
    skill::{Skill, SkillKind, SkillState},
};
#[cfg(test)]
use crate::{Settings, bridge::MockInput, detect::MockDetector};
#[double]
use crate::{navigator::Navigator, rotator::Rotator};

/// The FPS the bot runs at.
///
/// This must **not** be changed as it affects other ticking systems.
const FPS: u32 = 30;

/// Seconds to wait before halting.
const PENDING_HALT_SECS: u64 = 12;

/// Milliseconds per tick as an [`u64`].
pub const MS_PER_TICK: u64 = MS_PER_TICK_F32 as u64;

/// Milliseconds per tick as an [`f32`].
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
    /// A struct to send inputs.
    pub input: Box<dyn Input>,
    /// A struct for generating random values.
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
    /// Whether minimap changed to detecting on current tick.
    pub tick_changed_minimap: bool,
    /// Whether capturing starts failing on current tick.
    tick_failed_capturing: bool,
}

impl Context {
    #[cfg(test)]
    pub fn new(input: Option<MockInput>, detector: Option<MockDetector>) -> Self {
        Context {
            input: Box::new(input.unwrap_or_default()),
            rng: Rng::new(rand::random()),
            notification: DiscordNotification::new(Rc::new(RefCell::new(Settings::default()))),
            detector: detector.map(|detector| Box::new(detector) as Box<dyn Detector>),
            minimap: Minimap::Detecting,
            player: Player::Detecting,
            skills: [Skill::Detecting; SkillKind::COUNT],
            buffs: [Buff::No; BuffKind::COUNT],
            operation: Operation::Running,
            tick: 0,
            tick_changed_minimap: false,
            tick_failed_capturing: false,
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

/// Current operating state of the bot.
#[derive(Debug, Clone, Copy)]
pub enum Operation {
    HaltUntil {
        instant: Instant,
        run_duration_millis: u64,
        stop_duration_millis: u64,
    },
    Halting,
    Running,
    RunUntil {
        instant: Instant,
        run_duration_millis: u64,
        stop_duration_millis: u64,
        once: bool,
    },
}

impl Operation {
    #[inline]
    pub fn halting(&self) -> bool {
        matches!(self, Operation::Halting | Operation::HaltUntil { .. })
    }

    pub fn update_current(
        self,
        cycle_run_stop: CycleRunStopMode,
        run_duration_millis: u64,
        stop_duration_millis: u64,
    ) -> Operation {
        match self {
            Operation::HaltUntil { .. } => match cycle_run_stop {
                CycleRunStopMode::None | CycleRunStopMode::Once => Operation::Halting,
                CycleRunStopMode::Repeat => {
                    let duration = Duration::from_millis(stop_duration_millis);
                    let instant = Instant::now() + duration;

                    Operation::HaltUntil {
                        instant,
                        run_duration_millis,
                        stop_duration_millis,
                    }
                }
            },
            Operation::Halting => Operation::Halting,
            Operation::Running | Operation::RunUntil { .. } => match cycle_run_stop {
                CycleRunStopMode::None => Operation::Running,
                CycleRunStopMode::Once | CycleRunStopMode::Repeat => {
                    let duration = Duration::from_millis(run_duration_millis);
                    let instant = Instant::now() + duration;

                    Operation::RunUntil {
                        instant,
                        run_duration_millis,
                        stop_duration_millis,
                        once: matches!(cycle_run_stop, CycleRunStopMode::Once),
                    }
                }
            },
        }
    }

    fn update(self) -> Operation {
        match self {
            // Imply run/stop cycle enabled
            Operation::HaltUntil {
                instant,
                run_duration_millis,
                stop_duration_millis,
            } => {
                let now = Instant::now();
                if now < instant {
                    return self;
                }

                let duration = Duration::from_millis(run_duration_millis);
                let instant = now + duration;
                Operation::RunUntil {
                    instant,
                    run_duration_millis,
                    stop_duration_millis,
                    once: false,
                }
            }
            Operation::Halting => Operation::Halting,
            Operation::Running => Operation::Running,
            // Imply run/stop cycle enabled
            Operation::RunUntil {
                instant,
                run_duration_millis,
                stop_duration_millis,
                once,
            } => {
                let now = Instant::now();
                if now < instant {
                    return self;
                }
                if once {
                    return Operation::Halting;
                }

                let duration = Duration::from_millis(run_duration_millis);
                let instant = now + duration;
                Operation::HaltUntil {
                    instant,
                    run_duration_millis,
                    stop_duration_millis,
                }
            }
        }
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
        platforms::init();
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
    let settings = Rc::new(RefCell::new(query_settings()));
    let seeds = query_seeds(); // Fixed, unchanged
    let rng = Rng::new(seeds.seed); // Create one for Context
    let (mut service, keys, mut capture) = DefaultService::new(seeds, settings.clone());

    let mut rotator = Rotator::default();
    let mut navigator = Navigator::default();
    let mut context = Context {
        input: keys,
        rng,
        notification: DiscordNotification::new(settings.clone()),
        detector: None,
        minimap: Minimap::Detecting,
        player: Player::Idle,
        skills: [Skill::Detecting],
        buffs: [Buff::No; BuffKind::COUNT],
        operation: Operation::Halting,
        tick: 0,
        tick_changed_minimap: false,
        tick_failed_capturing: false,
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
    let mut did_capture_normally = false;

    loop_with_fps(FPS, || {
        let detector = capture
            .grab()
            .map(OwnedMat::new_from_frame)
            .map(CachedDetector::new);
        let was_player_alive = !player_state.is_dead();
        let was_player_navigating = navigator.was_last_point_available_or_completed();
        let was_running_cycle = matches!(context.operation, Operation::RunUntil { .. });
        let was_capturing_normally = did_capture_normally;

        did_capture_normally = detector.is_ok();
        context.tick += 1;
        context.tick_failed_capturing = was_capturing_normally
            && !did_capture_normally
            && matches!(
                detector.as_ref().err(),
                Some(Error::WindowNotFound | Error::WindowFrameNotAvailable)
            );
        context.operation = context.operation.update();

        let did_cycled_to_stop = context.operation.halting();
        if let Ok(detector) = detector {
            let was_minimap_idle = matches!(context.minimap, Minimap::Idle(_));

            context.detector = Some(Box::new(detector));
            context.minimap = fold_context(&context, context.minimap, &mut minimap_state);
            context.tick_changed_minimap =
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

        context.input.update(context.tick);
        context.notification.update_scheduled_frames(|| {
            to_png(context.detector.as_ref().map(|detector| detector.mat()))
        });

        // Poll requests, keys and update scheduled notifications frames
        service.poll(PollArgs {
            context: &mut context,
            player: &mut player_state,
            minimap: &mut minimap_state,
            buffs: &mut buff_states,
            rotator: &mut rotator,
            navigator: &mut navigator,
            capture: &mut capture,
        });

        // Go to town on stop cycle
        if was_running_cycle && did_cycled_to_stop {
            rotator.reset_queue();
            player_state.clear_actions_aborted(false);
            context.player = Player::Panicking(Panicking::new(PanicTo::Town));
        }

        // Upon accidental or white roomed causing map to change,
        // abort actions and send notification
        if service.has_minimap_data() && !context.operation.halting() {
            let player_died = was_player_alive && player_state.is_dead();
            // Unconditionally halt when player died
            if player_died {
                rotator.reset_queue();
                player_state.clear_actions_aborted(true);
                context.operation = Operation::Halting;
                return;
            }

            let stop_on_fail_or_change_map = settings.borrow().stop_on_fail_or_change_map;
            if !stop_on_fail_or_change_map {
                return;
            }

            let mut pending_halt_reached = pending_halt.is_some_and(|instant| {
                Instant::now().duration_since(instant).as_secs() >= PENDING_HALT_SECS
            });
            if context.tick_changed_minimap || (pending_halt_reached && was_player_navigating) {
                pending_halt_reached = false;
                pending_halt = None;
            }

            // Do not halt if player changed map due to switching channel
            let player_panicking = matches!(
                context.player,
                Player::Panicking(Panicking {
                    to: PanicTo::Channel,
                    ..
                })
            );
            let can_halt_or_notify = pending_halt_reached
                || (pending_halt.is_none() && context.tick_changed_minimap && !player_panicking)
                || (pending_halt.is_none() && context.tick_failed_capturing);
            if can_halt_or_notify {
                if pending_halt.is_none() {
                    pending_halt = Some(Instant::now());
                } else {
                    rotator.reset_queue();
                    context.operation = Operation::Halting;
                    if did_capture_normally {
                        context.player = Player::Panicking(Panicking::new(PanicTo::Town));
                    }
                    player_state.clear_actions_aborted(!did_capture_normally);
                    pending_halt = None;
                }

                if pending_halt.is_none() {
                    let _ = context
                        .notification
                        .schedule_notification(NotificationKind::FailOrMapChange);
                }
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
