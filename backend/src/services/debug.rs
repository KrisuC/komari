use std::{sync::LazyLock, time::Instant};

use include_dir::{Dir, include_dir};
use log::debug;
use opencv::{
    core::{Mat, ModifyInplace, Vector},
    imgcodecs::{IMREAD_COLOR, imdecode},
    imgproc::{COLOR_BGR2BGRA, cvt_color_def},
};
use rand::distr::SampleString;
use rand_distr::Alphanumeric;

use crate::{
    context::Context,
    debug::{save_image_for_training, save_image_for_training_to, save_minimap_for_training},
    detect::{ArrowsCalibrating, ArrowsState, CachedDetector, Detector},
    mat::OwnedMat,
};

const SOLVE_RUNE_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Default)]
pub struct DebugService {
    recording_id: Option<String>,
    infering_rune: Option<(ArrowsCalibrating, Instant)>,
}

impl DebugService {
    pub fn poll(&mut self, context: &Context) {
        if let Some(id) = self.recording_id.clone() {
            save_image_for_training_to(context.detector_unwrap().mat(), Some(id), false, false);
        }

        if let Some((calibrating, instant)) = self.infering_rune.as_ref().copied() {
            if instant.elapsed().as_secs() >= SOLVE_RUNE_TIMEOUT_SECS {
                self.infering_rune = None;
                debug!(target: "debug", "infer rune timed out");
                return;
            }

            match context.detector_unwrap().detect_rune_arrows(calibrating) {
                Ok(ArrowsState::Complete(arrows)) => {
                    // TODO: Save
                    self.infering_rune = None;
                    debug!(target: "debug", "infer rune result {arrows:?}");
                }
                Ok(ArrowsState::Calibrating(calibrating)) => {
                    self.infering_rune = Some((calibrating, instant));
                }
                Err(err) => {
                    self.infering_rune = None;
                    debug!(target: "debug", "infer rune failed {err}");
                }
            }
        }
    }

    pub fn set_auto_save_rune(&self, context: &Context, auto_save: bool) {
        context.debug.set_auto_save_rune(auto_save);
    }

    pub fn capture_image(&self, context: &Context, is_grayscale: bool) {
        if let Some(detector) = context.detector.as_ref() {
            save_image_for_training(detector.mat(), is_grayscale, false);
        }
    }

    pub fn record_images(&mut self, start: bool) {
        self.recording_id = if start {
            Some(Alphanumeric.sample_string(&mut rand::rng(), 8))
        } else {
            None
        };
    }

    pub fn infer_rune(&mut self) {
        self.infering_rune = Some((ArrowsCalibrating::default(), Instant::now()));
    }

    pub fn infer_minimap(&self, context: &Context) {
        if let Some(detector) = context.detector.as_ref()
            && let Some(bbox) = detector.detect_minimap(160).ok()
        {
            save_minimap_for_training(detector.mat(), bbox);
        }
    }

    pub fn test_spin_rune(&self) {
        static SPIN_TEST_DIR: Dir<'static> = include_dir!("$SPIN_TEST_DIR");
        static SPIN_TEST_IMAGES: LazyLock<Vec<Mat>> = LazyLock::new(|| {
            let mut files = SPIN_TEST_DIR.files().collect::<Vec<_>>();
            files.sort_by_key(|file| file.path().to_str().unwrap());
            files
                .into_iter()
                .map(|file| {
                    let vec = Vector::from_slice(file.contents());
                    let mut mat = imdecode(&vec, IMREAD_COLOR).unwrap();
                    unsafe {
                        mat.modify_inplace(|mat, mat_mut| {
                            cvt_color_def(mat, mat_mut, COLOR_BGR2BGRA).unwrap();
                        });
                    }
                    mat
                })
                .collect()
        });

        let mut calibrating = ArrowsCalibrating::default();
        calibrating.enable_spin_test();

        for mat in &*SPIN_TEST_IMAGES {
            match CachedDetector::new(OwnedMat::from(mat.clone())).detect_rune_arrows(calibrating) {
                Ok(ArrowsState::Complete(arrows)) => {
                    debug!(target: "test", "spin test completed {arrows:?}");
                }
                Ok(ArrowsState::Calibrating(new_calibrating)) => {
                    calibrating = new_calibrating;
                }
                Err(err) => {
                    debug!(target: "test", "spin test error {err}");
                    break;
                }
            }
        }
    }
}
