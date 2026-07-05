use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(debug_assertions)]
use opencv::{
    core::Size,
    videoio::{VideoWriter, VideoWriterTrait},
};

use crate::services::Event;
#[cfg(test)]
use crate::{Settings, bridge::MockInput, detect::MockDetector};
use crate::{
    bridge::Input, buff::BuffEntities, detect::Detector, minimap::MinimapEntity,
    notification::Notification, operation::Operation, player::PlayerEntity, rng::Rng,
    skill::SkillEntities,
};
#[cfg(debug_assertions)]
use crate::{debug::save_rune_for_training, run::FPS, solvers::SolvedArrow, utils::DatasetDir};

#[derive(Debug)]
#[cfg(debug_assertions)]
pub struct RecordingHandle {
    writer: VideoWriter,
}

#[cfg(debug_assertions)]
impl RecordingHandle {
    pub fn write(&mut self, detector: &dyn Detector) {
        self.writer.write(&detector.mat()).unwrap()
    }
}

#[derive(Debug, Default)]
#[cfg(debug_assertions)]
pub struct Debug {
    pub auto_save_rune: bool,
    pub auto_record_lie_detector: bool,
    last_rune_detector: Option<Arc<dyn Detector>>,
    last_rune_result: Option<[SolvedArrow; 4]>,
}

#[cfg(debug_assertions)]
impl Debug {
    pub fn new_recording(&self, frame_size: Size) -> RecordingHandle {
        use rand::distr::SampleString;
        use rand_distr::Alphanumeric;

        let id = Alphanumeric.sample_string(&mut rand::rng(), 8);
        let file = DatasetDir::Recordings
            .to_folder()
            .join(format!("ld_{id}.mp4"));
        let fourcc = VideoWriter::fourcc('H', 'V', 'C', '1').unwrap();
        let writer =
            VideoWriter::new(file.to_str().unwrap(), fourcc, FPS as f64, frame_size, true).unwrap();

        RecordingHandle { writer }
    }

    pub fn save_last_rune_result(&self) {
        if !self.auto_save_rune {
            return;
        }

        if let Some((detector, result)) =
            self.last_rune_detector.as_ref().zip(self.last_rune_result)
        {
            save_rune_for_training(&detector.mat(), result);
        }
    }

    pub fn set_last_rune_result(&mut self, detector: Arc<dyn Detector>, result: [SolvedArrow; 4]) {
        self.last_rune_detector = Some(detector);
        self.last_rune_result = Some(result);
    }
}

/// A struct containing shared resources.
///
/// TODO: Reduce field visibilities.
#[derive(Debug)]
pub struct Resources {
    /// A resource to hold debugging information.
    #[cfg(debug_assertions)]
    pub debug: Debug,
    /// A resource to send inputs.
    pub input: Box<dyn Input>,
    /// A resource for generating random values.
    pub rng: Rng,
    /// A resource for sending notifications through web hook.
    pub notification: Notification,
    /// A resource to detect game information.
    ///
    /// This is [`None`] when no frame as ever been captured.
    pub detector: Option<Arc<dyn Detector>>,
    /// A resource indicating current operation state.
    pub operation: Operation,
    /// A resource indicating current tick.
    pub tick: u64,
    /// How many lie detectors have been encountered so far.
    pub lie_detector_count: u64,
    /// Accumulated time the bot has been running since startup.
    pub total_runtime: Duration,
    /// Tracks when the last runtime accumulation happened.
    pub last_runtime_tick: Instant,
}

impl Resources {
    #[cfg(test)]
    pub fn new(input: Option<MockInput>, detector: Option<MockDetector>) -> Self {
        use std::{cell::RefCell, rc::Rc};

        use crate::operation::{OperationConfiguration, OperationState};

        Self {
            #[cfg(debug_assertions)]
            debug: Debug::default(),
            input: Box::new(input.unwrap_or_default()),
            rng: Rng::new(rand::random(), rand::random()),
            notification: Notification::new(Rc::new(RefCell::new(Settings::default()))),
            detector: detector.map(|detector| Arc::new(detector) as Arc<dyn Detector>),
            operation: Operation {
                config: OperationConfiguration {
                    run_timer: false,
                    run_timer_millis: 0,
                },
                state: OperationState::Running,
            },
            tick: 0,
            lie_detector_count: 0,
            total_runtime: Duration::ZERO,
            last_runtime_tick: Instant::now(),
        }
    }

    /// Retrieves a reference to a [`Detector`] for the latest captured frame.
    ///
    /// # Panics
    ///
    /// Panics if no frame has ever been captured.
    #[inline]
    pub fn detector(&self) -> &dyn Detector {
        self.detector
            .as_ref()
            .expect("detector is not available because no frame has ever been captured")
            .as_ref()
    }

    /// Same as [`Self::detector`] but cloned.
    #[inline]
    pub fn detector_cloned(&self) -> Arc<dyn Detector> {
        self.detector
            .as_ref()
            .cloned()
            .expect("detector is not available because no frame has ever been captured")
    }
}

/// Different game-related events.
#[derive(Debug, Clone, Copy)]
pub enum WorldEvent {
    RunTimerEnded,
    PlayerDied,
    MinimapChanged,
    CaptureFailed,
    LieDetectorShapeAppeared,
    LieDetectorViolettaAppeared,
    EliteBossAppeared,
}

impl Event for WorldEvent {}

/// A container for entities.
#[derive(Debug)]
pub struct World {
    pub minimap: MinimapEntity,
    pub player: PlayerEntity,
    pub skills: SkillEntities,
    pub buffs: BuffEntities,
}
