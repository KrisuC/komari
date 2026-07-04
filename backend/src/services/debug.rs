use std::{
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
    thread::sleep,
    time::{Duration, Instant},
};

use include_dir::{Dir, include_dir};
use opencv::{
    core::{copy_make_border, BorderTypes, Mat, MatTraitConst, ModifyInplace, Rect, Scalar, Vector},
    highgui::{destroy_all_windows, imshow, wait_key},
    imgcodecs::{IMREAD_COLOR, imdecode},
    imgproc::{COLOR_BGR2BGRA, cvt_color_def},
    videoio::{VideoCapture, VideoCaptureTrait, VideoCaptureTraitConst, VideoWriter, VideoWriterTrait},
};
use platforms::Window;
use rand::distr::SampleString;
use rand_distr::Alphanumeric;
use tokio::{
    sync::{
        broadcast::{self, Receiver, Sender},
        mpsc::{self},
    },
    task::spawn_blocking,
};

use crate::{
    DebugState, TransparentShapeDifficulty,
    bridge::{Input, MouseKind},
    detect::{DefaultDetector, Detector},
    ecs::Resources,
    mat::OwnedMat,
    models::Localization,
    run::FPS,
    solvers::{RuneSolver, TransparentShapeSolver, ViolettaSolver},
    utils::DatasetDir,
};

#[derive(Debug)]
pub struct DebugService {
    state: Sender<DebugState>,
    writer: Option<VideoWriter>,
}

impl Default for DebugService {
    fn default() -> Self {
        Self {
            state: broadcast::channel(1).0,
            writer: None,
        }
    }
}

impl DebugService {
    pub fn poll(&mut self, resources: &mut Resources) {
        if let Some(writer) = self.writer.as_mut()
            && let Some(detector) = resources.detector.as_ref()
        {
            writer.write(&detector.mat()).unwrap();
        }

        if self.state.is_empty() {
            let _ = self.state.send(DebugState {
                is_recording: self.writer.is_some(),
                is_rune_auto_saving: resources.debug.auto_save_rune,
                is_lie_detector_auto_recording: resources.debug.auto_record_lie_detector,
            });
        }
    }

    pub fn subscribe_state(&self) -> Receiver<DebugState> {
        self.state.subscribe()
    }

    pub fn record_video(&mut self, resources: &mut Resources, start: bool) {
        if !start {
            self.writer = None;
            return;
        }

        if resources.detector.is_none() {
            return;
        }

        let detector = resources.detector();
        let frame_size = detector.mat().size().unwrap();

        let id = Alphanumeric.sample_string(&mut rand::rng(), 8);
        let file = DatasetDir::Recordings.to_folder().join(format!("{id}.mp4"));
        let fourcc = VideoWriter::fourcc('H', 'V', 'C', '1').unwrap();

        let mut writer =
            VideoWriter::new(file.to_str().unwrap(), fourcc, FPS as f64, frame_size, true).unwrap();
        writer.write(&detector.mat()).unwrap();

        self.writer = Some(writer);
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
                    convert_bgr_to_bgra(&mut mat);
                    mat
                })
                .collect()
        });

        spawn_blocking(move || {
            let mut solver = RuneSolver::debug();
            for detector in SPIN_TEST_IMAGES
                .clone()
                .into_iter()
                .map(OwnedMat::from)
                .map(|mat| DefaultDetector::new(mat, Arc::new(Localization::default())))
            {
                solver.solve(&detector);
            }
            destroy_all_windows().unwrap();
        });
    }

    pub fn test_transparent_shape(
        &self,
        input: Box<dyn Input>,
        difficulty: TransparentShapeDifficulty,
    ) {
        static NORMAL_VIDEO: &[u8] = include_bytes!(env!("TRANSPARENT_SHAPE_TEST_NORMAL_VIDEO"));
        static HARD_VIDEO: &[u8] = include_bytes!(env!("TRANSPARENT_SHAPE_TEST_HARD_VIDEO"));

        let (name, video) = match difficulty {
            TransparentShapeDifficulty::Normal => {
                ("transparent_shape_test_normal.mp4", NORMAL_VIDEO)
            }
            TransparentShapeDifficulty::Hard => ("transparent_shape_test_hard.mp4", HARD_VIDEO),
        };
        let file = DatasetDir::Root.to_folder().join(name);
        if !file.exists() {
            let _ = fs::write(&file, video);
        }

        self.run_transparent_shape_test(input, file);
    }

    pub fn test_transparent_shape_file(&self, input: Box<dyn Input>, path: PathBuf) {
        log::info!("[debug_service] test_transparent_shape_file called with: {:?}", path);
        // Mirror the flow of test_transparent_shape: copy to dataset dir first.
        // This avoids any path encoding issues from the WebView2 file picker.
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("test_video.mp4");
        let dataset_file = DatasetDir::Root.to_folder().join(file_name);
        log::info!(
            "[debug_service] copying video to dataset dir: {:?}",
            dataset_file
        );
        match fs::copy(&path, &dataset_file) {
            Ok(bytes) => log::info!(
                "[debug_service] copied {} bytes from {:?} to {:?}",
                bytes,
                path,
                dataset_file
            ),
            Err(err) => {
                log::error!("[debug_service] failed to copy video: {:?}", err);
            }
        }
        self.run_transparent_shape_test(input, dataset_file);
    }

    fn run_transparent_shape_test(&self, mut input: Box<dyn Input>, file: PathBuf) {
        log::info!(
            "[debug_service] run_transparent_shape_test starting with: {:?}",
            file
        );
        spawn_blocking(move || {
            log::info!(
                "[debug_service::spawn_blocking] thread started for: {:?}",
                file
            );
            let mut frame_rx = frame_receiver_from_video(file);
            let mut solver = TransparentShapeSolver::debug();
            let localization = Arc::new(Localization::default());

            input.set_window(Window::new("Main HighGUI"));

            log::info!("[debug_service::spawn_blocking] entering frame loop");
            let mut frame_count = 0u64;
            let mut tracking_active = false;
            loop {
                if frame_rx.is_closed() {
                    log::info!(
                        "[debug_service] test ended after {} frames (channel closed)",
                        frame_count
                    );
                    return;
                }

                if let Ok(mut frame) = frame_rx.try_recv() {
                    // If the frame is a cropped/trimmed video (much smaller than a full
                    // game screen), pad it to full game resolution. The YOLO model was
                    // trained on full game screens (~1366x768), so a cropped frame alone
                    // produces near-zero confidence scores.
                    let is_cropped = frame.cols() < 1000 || frame.rows() < 700;
                    if is_cropped {
                        frame = pad_frame_to_game_screen(frame);
                    }

                    if frame_count == 0 {
                        log::info!(
                            "First frame received: {}x{} (cropped={}), showing window...",
                            frame.cols(),
                            frame.rows(),
                            is_cropped,
                        );
                    }
                    frame_count += 1;
                    let region = Rect::new(0, 0, frame.cols(), frame.rows());
                    let detector =
                        DefaultDetector::new(OwnedMat::from(frame), localization.clone());
                    let cursor = solver.solve(&detector, region);

                    // The solver's debug_transparent_shapes handles display when tracking
                    // is active (draws bounding boxes + arrows, calls imshow).
                    // When tracking is NOT active, we show the raw frame so the video
                    // is still visible. Once tracking starts, we let the solver own
                    // the display to avoid flickering from double-imshow.
                    if cursor.is_some() {
                        tracking_active = true;
                    }
                    if !tracking_active {
                        let _ = imshow("Shape Tracks", &detector.mat());
                    }
                    // Always pump HighGUI events to keep window responsive.
                    let _ = wait_key(1);

                    if frame_count % 30 == 0 {
                        log::info!(
                            "[debug_service] processed {} frames, last cursor: {:?}",
                            frame_count,
                            cursor
                        );
                    }

                    if let Some(cursor) = cursor {
                        input.send_mouse(cursor.x, cursor.y, MouseKind::Move);
                    }
                }
            }
        });
    }

    pub fn test_violetta(&self, mut input: Box<dyn Input>) {
        static VIDEO: &[u8] = include_bytes!(env!("VIOLETTA_TEST_VIDEO"));

        spawn_blocking(move || {
            let file = DatasetDir::Root.to_folder().join("violetta_test.mp4");
            if !file.exists() {
                let _ = fs::write(&file, VIDEO);
            }

            let mut frame_rx = frame_receiver_from_video(file);
            let mut solver = ViolettaSolver::debug();
            let localization = Arc::new(Localization::default());

            input.set_window(Window::new("Main HighGUI"));

            loop {
                if frame_rx.is_closed() {
                    return;
                }

                if let Ok(frame) = frame_rx.try_recv() {
                    let region = Rect::new(0, 0, frame.cols(), frame.rows());
                    let detector =
                        DefaultDetector::new(OwnedMat::from(frame), localization.clone());
                    if let Some(cursor) = solver.solve(&detector, region) {
                        input.send_mouse(cursor.x, cursor.y, MouseKind::Move);
                    }
                }
            }
        });
    }
}

fn frame_receiver_from_video(file: PathBuf) -> mpsc::Receiver<Mat> {
    fn read_and_send_frame(capture: &mut VideoCapture, tx: &mpsc::Sender<Mat>) -> bool {
        let mut frame = Mat::default();
        if !capture.read(&mut frame).unwrap_or(false) {
            log::info!("[frame_reader] read_and_send_frame: read failed (end of video or decode error)");
            return false;
        }

        convert_bgr_to_bgra(&mut frame);
        let _ = tx.try_send(frame);

        true
    }

    let (tx, rx) = mpsc::channel(3);

    let path_str = match file.to_str() {
        Some(s) => s,
        None => {
            log::error!("video path is not valid UTF-8: {:?}", file);
            return rx;
        }
    };

    let mut capture = match VideoCapture::from_file_def(path_str) {
        Ok(cap) => {
            let fps = cap.get(opencv::videoio::CAP_PROP_FPS).unwrap_or(0.0);
            let w = cap.get(opencv::videoio::CAP_PROP_FRAME_WIDTH).unwrap_or(0.0);
            let h = cap.get(opencv::videoio::CAP_PROP_FRAME_HEIGHT).unwrap_or(0.0);
            log::info!(
                "Opened video '{}': {:.0}x{:.0} @ {:.2} fps",
                path_str, w, h, fps
            );
            cap
        }
        Err(err) => {
            log::error!("failed to open video '{}': {:?}", path_str, err);
            return rx;
        }
    };

    spawn_blocking(move || {
        log::info!(
            "[frame_reader] thread started, reading frames at {} FPS",
            FPS
        );
        loop_with_fps(FPS, || read_and_send_frame(&mut capture, &tx));
        log::info!("[frame_reader] thread finished (video ended or read error)");
    });

    rx
}

fn convert_bgr_to_bgra(frame: &mut Mat) {
    unsafe {
        frame.modify_inplace(|src, dst| {
            cvt_color_def(src, dst, COLOR_BGR2BGRA).expect("color conversion failed");
        });
    }
}

fn loop_with_fps(fps: u32, mut on_tick: impl FnMut() -> bool) {
    let nanos_per_frame = (1_000_000_000 / fps) as u128;
    loop {
        let start = Instant::now();

        if !on_tick() {
            return;
        }

        let now = Instant::now();
        let elapsed_duration = now.duration_since(start);
        let elapsed_nanos = elapsed_duration.as_nanos();
        if elapsed_nanos <= nanos_per_frame {
            sleep(Duration::new(0, (nanos_per_frame - elapsed_nanos) as u32));
        }
    }
}

/// Pads a cropped/trimmed frame to a standard game screen resolution.
///
/// The YOLO model was trained on full game screens (~1366×768). A frame that
/// contains only the cropped detection region produces near-zero confidence
/// scores because the model expects the full game context. Padding the frame
/// with black borders to match the training resolution lets the model see the
/// shapes at the expected scale and position.
fn pad_frame_to_game_screen(frame: Mat) -> Mat {
    // Standard game resolution the model was trained on
    let target_w = 1366i32;
    let target_h = 768i32;

    let frame_w = frame.cols();
    let frame_h = frame.rows();

    // If the frame is already large enough, return as-is
    if frame_w >= target_w && frame_h >= target_h {
        return frame;
    }

    // Center the cropped frame within the game screen canvas.
    // The model was trained on full game screens where the detection region
    // is roughly centered. Centering ensures shapes are at the expected
    // position regardless of the exact crop coordinates used.
    let top = (target_h - frame_h) / 2;
    let bottom = target_h - top - frame_h;
    let left = (target_w - frame_w) / 2;
    let right = target_w - left - frame_w;

    let mut result = Mat::default();
    let _ = copy_make_border(
        &frame,
        &mut result,
        top.max(0),
        bottom.max(0),
        left.max(0),
        right.max(0),
        BorderTypes::BORDER_CONSTANT as i32,
        Scalar::all(0.0),
    );

    result
}
