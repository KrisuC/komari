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
#[cfg(debug_assertions)]
use opencv::core::Rect;
use opencv::{
    core::{Vector, VectorToVec},
    imgcodecs::imencode_def,
};
use strum::IntoEnumIterator;
use tokio::sync::broadcast::channel;

#[cfg(debug_assertions)]
use crate::bridge::KeyKind;
#[cfg(debug_assertions)]
use crate::debug::save_rune_for_training;
use crate::{
    CycleRunStopMode,
    bridge::{Capture, Input},
    buff::{Buff, BuffKind, BuffState},
    database::{query_seeds, query_settings},
    detect::{CachedDetector, Detector},
    mat::OwnedMat,
    minimap::{Minimap, MinimapState},
    navigator::{DefaultNavigator, Navigator},
    notification::DiscordNotification,
    player::{Player, PlayerState},
    rng::Rng,
    rotator::{DefaultRotator, Rotator},
    services::{DefaultService, PollArgs},
    skill::{Skill, SkillKind, SkillState},
};
#[cfg(test)]
use crate::{Settings, bridge::MockInput, detect::MockDetector};

/// The FPS the bot runs at.
///
/// This must **not** be changed as it affects other ticking systems.
const FPS: u32 = 30;

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

#[derive(Debug, Default)]
#[cfg(debug_assertions)]
pub struct Debug {
    auto_save: RefCell<bool>,
    last_rune_detector: RefCell<Option<Box<dyn Detector>>>,
    last_rune_result: RefCell<Option<[(Rect, KeyKind); 4]>>,
}

#[cfg(debug_assertions)]
impl Debug {
    pub fn auto_save_rune(&self) -> bool {
        *self.auto_save.borrow()
    }

    pub fn set_auto_save_rune(&self, auto_save: bool) {
        *self.auto_save.borrow_mut() = auto_save;
    }

    pub fn save_last_rune_result(&self) {
        if !*self.auto_save.borrow() {
            return;
        }
        if let Some((detector, result)) = self
            .last_rune_detector
            .borrow()
            .as_ref()
            .zip(*self.last_rune_result.borrow())
        {
            save_rune_for_training(detector.mat(), result);
        }
    }

    pub fn set_last_rune_result(&self, detector: Box<dyn Detector>, result: [(Rect, KeyKind); 4]) {
        *self.last_rune_detector.borrow_mut() = Some(detector);
        *self.last_rune_result.borrow_mut() = Some(result);
    }
}

/// A struct that stores the game information.
#[derive(Debug)]
pub struct Context {
    /// A struct to hold debugging information.
    #[cfg(debug_assertions)]
    pub debug: Debug,
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
}

impl Context {
    #[cfg(test)]
    pub fn new(input: Option<MockInput>, detector: Option<MockDetector>) -> Self {
        Context {
            #[cfg(debug_assertions)]
            debug: Debug::default(),
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

#[derive(Debug, Clone, Copy)]
pub enum ContextEvent {
    CycledToHalt,
    PlayerDied,
    MinimapChanged,
    CaptureFailed,
}

/// Current operating state of the bot.
#[derive(Debug, Clone, Copy)]
pub enum Operation {
    HaltUntil {
        instant: Instant,
        run_duration_millis: u64,
        stop_duration_millis: u64,
    },
    TemporaryHalting {
        resume: Duration,
        run_duration_millis: u64,
        stop_duration_millis: u64,
        once: bool,
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
        matches!(
            self,
            Operation::Halting | Operation::HaltUntil { .. } | Operation::TemporaryHalting { .. }
        )
    }

    pub fn update_current(
        self,
        cycle_run_stop: CycleRunStopMode,
        run_duration_millis: u64,
        stop_duration_millis: u64,
    ) -> Operation {
        match self {
            Operation::HaltUntil {
                stop_duration_millis: current_stop_duration_millis,
                ..
            } => match cycle_run_stop {
                CycleRunStopMode::None | CycleRunStopMode::Once => Operation::Halting,
                CycleRunStopMode::Repeat => {
                    if current_stop_duration_millis == stop_duration_millis {
                        self
                    } else {
                        Operation::halt_until(run_duration_millis, stop_duration_millis)
                    }
                }
            },
            Operation::TemporaryHalting {
                run_duration_millis: current_run_duration_millis,
                ..
            } => {
                if current_run_duration_millis != run_duration_millis
                    || matches!(cycle_run_stop, CycleRunStopMode::None)
                {
                    Operation::Halting
                } else {
                    self
                }
            }
            Operation::Halting => Operation::Halting,
            Operation::Running | Operation::RunUntil { .. } => match cycle_run_stop {
                CycleRunStopMode::None => Operation::Running,
                CycleRunStopMode::Once | CycleRunStopMode::Repeat => Operation::run_until(
                    run_duration_millis,
                    stop_duration_millis,
                    matches!(cycle_run_stop, CycleRunStopMode::Once),
                ),
            },
        }
    }

    fn update(self) -> Operation {
        let now = Instant::now();
        match self {
            // Imply run/stop cycle enabled
            Operation::HaltUntil {
                instant,
                run_duration_millis,
                stop_duration_millis,
            } => {
                if now < instant {
                    self
                } else {
                    Operation::run_until(run_duration_millis, stop_duration_millis, false)
                }
            }
            // Imply run/stop cycle enabled
            Operation::RunUntil {
                instant,
                run_duration_millis,
                stop_duration_millis,
                once,
            } => {
                if now < instant {
                    self
                } else if once {
                    Operation::Halting
                } else {
                    Operation::halt_until(run_duration_millis, stop_duration_millis)
                }
            }
            Operation::Halting | Operation::TemporaryHalting { .. } | Operation::Running => self,
        }
    }

    #[inline]
    fn halt_until(run_duration_millis: u64, stop_duration_millis: u64) -> Operation {
        Operation::HaltUntil {
            instant: Instant::now() + Duration::from_millis(stop_duration_millis),
            run_duration_millis,
            stop_duration_millis,
        }
    }

    #[inline]
    pub fn run_until(run_duration_millis: u64, stop_duration_millis: u64, once: bool) -> Operation {
        Operation::RunUntil {
            instant: Instant::now() + Duration::from_millis(run_duration_millis),
            run_duration_millis,
            stop_duration_millis,
            once,
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
    let (event_tx, event_rx) = channel::<ContextEvent>(5);
    let (mut service, keys, mut capture) =
        DefaultService::new(seeds, settings.clone(), event_tx.subscribe());

    let mut rotator = DefaultRotator::default();
    let mut navigator = DefaultNavigator::new(event_rx);
    let mut context = Context {
        #[cfg(debug_assertions)]
        debug: Debug::default(),
        input: Box::new(keys),
        rng,
        notification: DiscordNotification::new(settings.clone()),
        detector: None,
        minimap: Minimap::Detecting,
        player: Player::Idle,
        skills: [Skill::Detecting],
        buffs: [Buff::No; BuffKind::COUNT],
        operation: Operation::Halting,
        tick: 0,
    };
    let mut player_state = PlayerState::default();
    let mut minimap_state = MinimapState::default();
    let mut skill_states = SkillKind::iter()
        .map(SkillState::new)
        .collect::<Vec<SkillState>>();
    let mut buff_states = BuffKind::iter()
        .map(BuffState::new)
        .collect::<Vec<BuffState>>();
    let mut is_capturing_normally = false;

    loop_with_fps(FPS, || {
        let detector = capture
            .grab()
            .map(OwnedMat::new_from_frame)
            .map(CachedDetector::new);
        let was_capturing_normally = is_capturing_normally;

        is_capturing_normally = detector.is_ok();
        context.tick += 1;
        if let Ok(detector) = detector {
            let was_player_alive = !player_state.is_dead();
            let was_running_cycle = matches!(context.operation, Operation::RunUntil { .. });
            let was_minimap_idle = matches!(context.minimap, Minimap::Idle(_));

            context.operation = context.operation.update();
            context.detector = Some(Box::new(detector));
            context.minimap = fold_context(&context, context.minimap, &mut minimap_state);
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

            if navigator.navigate_player(&context, &mut player_state) {
                rotator.rotate_action(&context, &mut player_state);
            }

            let did_cycled_to_stop = context.operation.halting();
            // Go to town on stop cycle
            if was_running_cycle && did_cycled_to_stop {
                let _ = event_tx.send(ContextEvent::CycledToHalt);
            }

            let player_died = was_player_alive && player_state.is_dead();
            if player_died {
                let _ = event_tx.send(ContextEvent::PlayerDied);
            }

            let minimap_detecting = matches!(context.minimap, Minimap::Detecting);
            if was_minimap_idle && minimap_detecting {
                let _ = event_tx.send(ContextEvent::MinimapChanged);
            }
        }

        if was_capturing_normally && !is_capturing_normally {
            let _ = event_tx.send(ContextEvent::CaptureFailed);
        }

        context.input.update(context.tick);
        context
            .notification
            .update(|| to_png(context.detector.as_ref().map(|detector| detector.mat())));
        service.poll(PollArgs {
            context: &mut context,
            player: &mut player_state,
            minimap: &mut minimap_state,
            buffs: &mut buff_states,
            rotator: &mut rotator,
            navigator: &mut navigator,
            capture: &mut capture,
        });
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
