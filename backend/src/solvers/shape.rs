use std::{
    collections::{HashMap, VecDeque},
    ops::Div,
    sync::atomic::{AtomicU64, Ordering},
};

use log::{debug, info};
use opencv::{
    core::{Mat, MatTraitConst, Point, Point_, Point2d, Rect, ToInputArray},
    imgproc,
};

use crate::{
    detect::Detector,
    run::FPS,
    tracker::{ByteTracker, Detection, IouGating, STrack},
};

/// Frames to use slow cursor transition after a distant track switch.
const CURSOR_SMOOTHING_SLOW_FRAMES: u32 = 10;

/// Sliding-window size for orientation-angle history used in rotation detection.
const ANGLE_WINDOW: usize = 20;

#[derive(Debug)]
pub struct TransparentShapeSolver {
    tracker: ByteTracker,
    current_track_id: Option<u64>,
    candidate_track_id: Option<u64>,
    candidate_track_count: u32,
    last_cursor: Option<Point>,
    last_velocity: Option<Point2d>,
    bg_direction: Point2d,
    /// How many consecutive frames the current track has been tracked.
    current_track_tenure: u32,
    /// Countdown of frames to use slow cursor smoothing after a track switch.
    slow_smoothing_remaining: u32,
    /// Per-track orientation-angle history for rotation detection.
    /// The correct shape self-rotates, causing its principal-axis angle
    /// (computed via image moments) to drift. Distractors only translate,
    /// so their orientation stays nearly constant.
    angle_history: HashMap<u64, VecDeque<f64>>,
    #[cfg(debug_assertions)]
    is_debugging: bool,
}

impl Default for TransparentShapeSolver {
    fn default() -> Self {
        Self {
            tracker: ByteTracker::new(FPS as u64, 0.25, 0.1, 0.25, IouGating::None),
            current_track_id: None,
            candidate_track_id: None,
            candidate_track_count: 0,
            last_cursor: None,
            last_velocity: None,
            bg_direction: Point2d::default(),
            current_track_tenure: 0,
            slow_smoothing_remaining: 0,
            angle_history: HashMap::new(),
            #[cfg(debug_assertions)]
            is_debugging: false,
        }
    }
}

impl TransparentShapeSolver {
    #[cfg(debug_assertions)]
    pub fn debug() -> Self {
        // Use very permissive tracker thresholds for debug/testing mode.
        // The user's video may have different visual characteristics than the
        // training data, resulting in lower confidence scores. Lowering thresholds
        // ensures the tracker doesn't filter out all detections.
        Self {
            tracker: ByteTracker::new(
                FPS as u64, // max_time_lost
                0.05,       // high_match_score_threshold (was 0.25)
                0.01,       // low_match_score_threshold  (was 0.1)
                0.05,       // new_track_score_threshold  (was 0.25)
                IouGating::None,
            ),
            current_track_id: None,
            candidate_track_id: None,
            candidate_track_count: 0,
            last_cursor: None,
            last_velocity: None,
            bg_direction: Point2d::default(),
            current_track_tenure: 0,
            slow_smoothing_remaining: 0,
            angle_history: HashMap::new(),
            is_debugging: true,
        }
    }

    pub fn solve(&mut self, detector: &dyn Detector, region: Rect) -> Option<Point> {
        let shapes = detector.detect_transparent_shapes(region);
        let shape_count = shapes.len();

        #[cfg(debug_assertions)]
        let top_scores: Vec<f32> = if self.is_debugging {
            let mut scores: Vec<f32> = shapes.iter().map(|(_, s)| *s).collect();
            scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            scores.into_iter().take(5).collect()
        } else {
            Vec::new()
        };

        let tracks = self.tracker.update(
            shapes
                .into_iter()
                .map(|(bbox, score)| Detection::new(bbox, score))
                .collect(),
        );

        #[cfg(debug_assertions)]
        if self.is_debugging {
            static CALL_COUNT: AtomicU64 = AtomicU64::new(0);
            let call = CALL_COUNT.fetch_add(1, Ordering::Relaxed);
            if call % 30 == 0 {
                info!(
                    "[solver] call#{} shapes={} tracks={} current_track={:?} last_cursor={:?} top_scores={:?}",
                    call,
                    shape_count,
                    tracks.len(),
                    self.current_track_id,
                    self.last_cursor,
                    top_scores,
                );
            }
        }

        // ── rotation detection via image-moments orientation ──
        // Compute each shape's principal-axis angle from its pixel
        // content. A self-rotating shape's angle drifts continuously;
        // a translating shape's angle stays stable. We track the
        // frame-to-frame angle deltas — the track with the largest
        // cumulative absolute drift is the rotating one.
        const ROTATION_CHECK_INTERVAL: u64 = 15;
        static SOLVE_COUNT: AtomicU64 = AtomicU64::new(0);
        let solve_n = SOLVE_COUNT.fetch_add(1, Ordering::Relaxed);

        // Only compute image moments every 2nd frame (~50% of the cost).
        // The angle history has enough samples even at 15 fps effective rate.
        if solve_n % 2 == 0 {
            if let Ok(region_mat) = detector.mat().roi(region) {
                for t in &tracks {
                    if let Ok(shape_roi) = region_mat.roi(t.rect()) {
                        if let Some(angle) = compute_principal_axis_angle(&shape_roi)
                        {
                            let history =
                                self.angle_history.entry(t.track_id()).or_default();
                            // Unwrap to avoid π-periodic jumps before storing.
                            if let Some(&prev) = history.back() {
                                let mut adjusted = angle;
                                while adjusted - prev > std::f64::consts::FRAC_PI_2
                                {
                                    adjusted -= std::f64::consts::PI;
                                }
                                while adjusted - prev < -std::f64::consts::FRAC_PI_2
                                {
                                    adjusted += std::f64::consts::PI;
                                }
                                history.push_back(adjusted);
                            } else {
                                history.push_back(angle);
                            }
                            if history.len() > ANGLE_WINDOW {
                                history.pop_front();
                            }
                        }
                    }
                }
            }
            // Purge dead tracks.
            let live_ids: Vec<u64> = tracks.iter().map(|t| t.track_id()).collect();
            self.angle_history.retain(|id, _| live_ids.contains(id));
        }

        self.update_initial_track_if_needed(region, &tracks);
        self.update_background_direction(&tracks);

        // Periodically check whether to switch to a track with a much
        // stronger rotation signal.
        if solve_n % ROTATION_CHECK_INTERVAL == 0
            && self.current_track_id.is_some()
        {
            let mut scores: Vec<(u64, f64)> = self
                .angle_history
                .iter()
                .filter_map(|(id, history)| {
                    if history.len() < ANGLE_WINDOW / 2 {
                        return None;
                    }
                    // Total absolute angle drift over the window.
                    let drift: f64 = history
                        .iter()
                        .zip(history.iter().skip(1))
                        .map(|(prev, curr)| (curr - prev).abs())
                        .sum();
                    Some((*id, drift))
                })
                .collect();

            scores.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap());

            if let Some(current_id) = self.current_track_id {
                let current_score = scores
                    .iter()
                    .find(|(id, _)| *id == current_id)
                    .map(|(_, s)| *s)
                    .unwrap_or(0.0);

                if let Some((best_id, best_score)) = scores.first() {
                    if *best_id != current_id
                        && *best_score > current_score * 2.0
                        && *best_score > 0.05
                        && self.current_track_tenure > ANGLE_WINDOW as u32
                    {
                        debug!(
                            target: "backend/player",
                            "shape id switches from {} to {} (angle drift: {:.4} vs {:.4})",
                            current_id, best_id, best_score, current_score
                        );
                        self.current_track_id = Some(*best_id);
                        self.current_track_tenure = 0;
                        self.slow_smoothing_remaining =
                            CURSOR_SMOOTHING_SLOW_FRAMES;
                    }
                }
            }
        }

        match self.update_and_find_best_track(&tracks, region) {
            Some(track) => {
                let cursor = mid_point(track.rect());
                let absolute = region.tl() + cursor;
                if self.current_track_id != Some(track.track_id()) {
                    debug!(target: "backend/player", "shape id switches from {:?} to {}", self.current_track_id, track.track_id());
                }
                self.current_track_id = Some(track.track_id());
                self.last_cursor = Some(cursor);
                self.last_velocity = Some(track.kalman_velocity());

                log::debug!(
                    target: "backend/player",
                    "solve: cursor=({}, {}) region=({},{}) bbox=({},{},{},{}) track_id={}",
                    absolute.x, absolute.y,
                    region.x, region.y,
                    track.rect().x, track.rect().y,
                    track.rect().width, track.rect().height,
                    track.track_id()
                );

                #[cfg(debug_assertions)]
                if self.is_debugging {
                    debug_transparent_shapes(
                        detector,
                        &tracks,
                        region,
                        cursor,
                        self.bg_direction,
                    );
                }

                Some(absolute)
            }
            None => {
                let last_cursor = self.last_cursor?;
                let last_velocity = self.last_velocity.expect("set if last_cursor set") * 1.5;
                let next_cursor = last_cursor
                    + Point::new(
                        last_velocity.x.round() as i32,
                        last_velocity.y.round() as i32,
                    );
                let absolute_next_cursor = region.tl() + next_cursor;
                if !region.contains(absolute_next_cursor) {
                    return None;
                }

                self.last_cursor = Some(next_cursor);

                Some(absolute_next_cursor)
            }
        }
    }

    /// Applies exponential smoothing to the raw cursor position.
    ///
    /// Each frame, the cursor moves toward the raw target. Normally 35% per
    /// frame (~88% in 5 frames). After a distant track switch, uses 12% for
    /// 10 frames to avoid a visible lurch.
    fn update_initial_track_if_needed(&mut self, region: Rect, tracks: &[STrack]) {
        if self.current_track_id.is_none() {
            // If we have enough aspect-ratio history, prefer the track
            // with the strongest rotation signal. Otherwise fall back
            // to closest-to-centre.
            let has_history = self
                .angle_history
                .values()
                .any(|h| h.len() >= ANGLE_WINDOW / 2);
            let picked = if has_history {
                tracks
                    .iter()
                    .max_by(|a, b| {
                        let va = self.compute_rotation_drift(a.track_id());
                        let vb = self.compute_rotation_drift(b.track_id());
                        va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
                    })
            } else {
                let region_mid =
                    mid_point(Rect::new(0, 0, region.width, region.height));
                find_track_closest_to(region_mid, tracks)
            };
            if let Some(track) = picked {
                self.current_track_id = Some(track.track_id());
                self.last_cursor = Some(mid_point(track.rect()));
                self.last_velocity = Some(track.kalman_velocity());
            }
        }
    }

    /// Returns the total absolute angle drift for `track_id` — the sum of
    /// frame-to-frame orientation changes over the history window. High
    /// drift = self-rotating shape; near-zero drift = translating shape.
    fn compute_rotation_drift(&self, track_id: u64) -> f64 {
        self.angle_history
            .get(&track_id)
            .map(|history| {
                if history.len() < 2 {
                    return 0.0;
                }
                history
                    .iter()
                    .zip(history.iter().skip(1))
                    .map(|(prev, curr)| (curr - prev).abs())
                    .sum::<f64>()
            })
            .unwrap_or(0.0)
    }

    fn update_background_direction(&mut self, tracks: &[STrack]) {
        if let Some(direction) = estimate_background_direction(self.last_cursor, tracks)
            .and_then(|direction| unit(self.bg_direction * 0.5 + direction * 0.5))
        {
            self.bg_direction = direction;
        }
    }

    fn update_and_find_best_track<'a>(
        &mut self,
        tracks: &'a [STrack],
        region: Rect,
    ) -> Option<&'a STrack> {
        let current_track_id = self.current_track_id?;
        let last_cursor = self.last_cursor?;
        let bg_direction = self.bg_direction;

        // Tenure bonus: the longer we've tracked the current shape, the
        // harder it is to switch away. Disabled for the first 15 frames
        // (grace period) so the solver can find the right shape at start.
        // After that, grows from 3.0x to 6.0x over 60 frames.
        let tenure_bonus = if self.current_track_tenure < 15 {
            1.0 // No bonus during grace period — allow free switching
        } else {
            ((self.current_track_tenure - 15) as f64)
                .min(60.0)
                .mul_add(0.05, 3.0)
        };

        let match_track = tracks
            .iter()
            .filter(|track| track.track_id() == current_track_id || track.tracklet_len() >= 1)
            .filter_map(|track| {
                let mut score =
                    track_background_score(track, last_cursor, bg_direction, region)?;
                if track.track_id() == current_track_id {
                    score *= tenure_bonus;
                }
                // Rotation bonus: shapes that self-rotate accumulate
                // larger total angle drift. Boost their score so the
                // solver naturally gravitates toward the rotating shape.
                let rot_drift = self.compute_rotation_drift(track.track_id());
                if rot_drift > 0.05 {
                    // Scale from 1.0× (no rotation) to ~4.0× (strong
                    // rotation). Typical drift for a rotating shape is
                    // 0.1–0.5 rad over a 20-frame window.
                    let rot_bonus = 1.0 + (rot_drift * 8.0).min(3.0);
                    score *= rot_bonus;
                }
                Some((track, score))
            })
            .max_by(|(_, a_score), (_, b_score)| a_score.partial_cmp(b_score).unwrap());

        if let Some((track, score)) = match_track {
            if track.track_id() == current_track_id {
                self.current_track_tenure += 1;
                self.candidate_track_id = None;
                self.candidate_track_count = 0;
                return Some(track);
            }

            // If the best candidate is a big jump with low penalized score,
            // don't switch — let the solver extrapolate from last known
            // position instead of jumping to an uncertain distant shape.
            if score < 0.15 {
                return None;
            }

            if self.candidate_track_id == Some(track.track_id()) {
                self.candidate_track_count += 1;
            } else {
                self.candidate_track_id = Some(track.track_id());
                self.candidate_track_count = 0;
            }

            // Need 5 consecutive frames as best before switching
            if self.candidate_track_count >= 4 {
                self.current_track_tenure = 0; // reset tenure for new track
                self.candidate_track_id = None;
                self.candidate_track_count = 0;
                self.slow_smoothing_remaining = CURSOR_SMOOTHING_SLOW_FRAMES;
                return Some(track);
            }
        }

        // Fallback: current track still in list but didn't win the scoring.
        // Still prefer it over switching to prevent oscillation.
        if let Some(current) = tracks.iter().find(|t| t.track_id() == current_track_id) {
            self.current_track_tenure += 1;
            return Some(current);
        }
        None
    }
}

impl Drop for TransparentShapeSolver {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        if self.is_debugging {
            use opencv::highgui::destroy_window;

            let _ = destroy_window("Shape Tracks");
        }
    }
}

#[cfg(debug_assertions)]
fn debug_transparent_shapes(
    detector: &dyn Detector,
    tracks: &[STrack],
    region: Rect,
    last_cursor: Point,
    bg_direction: Point2d,
) {
    use opencv::core::MatTraitConst;

    use crate::debug::debug_shape_tracks;

    debug_shape_tracks(
        &detector.mat().roi(region).unwrap(),
        tracks.to_vec(),
        last_cursor,
        bg_direction,
    );
}

fn find_track_closest_to(point: Point, tracks: &[STrack]) -> Option<&STrack> {
    tracks.iter().min_by_key(|track| {
        let track_region = track.rect();
        let track_mid =
            track_region.tl() + Point::new(track_region.width / 2, track_region.height / 2);

        (point - track_mid).norm() as i32
    })
}

fn mid_point(rect: Rect) -> Point {
    rect.tl() + Point::new(rect.width / 2, rect.height / 2)
}

fn track_background_score(
    track: &STrack,
    last_cursor: Point,
    bg_direction: Point2d,
    region: Rect,
) -> Option<f64> {
    let angle = track_background_degree(track, bg_direction)?;
    if angle <= 45.0 {
        return None;
    }
    let score = angle / 180.0;

    let distance_penalty = if angle >= 60.0 {
        1.0
    } else {
        let cursor_dir = mid_point(track.rect()) - last_cursor;
        let cursor_squared = (cursor_dir.x.pow(2) + cursor_dir.y.pow(2)) as f64;
        let sigma = 0.25 * diag(region);
        (-cursor_squared / (2.0 * sigma.powi(2))).exp()
    };
    if distance_penalty <= 0.3 {
        return None;
    }

    // Anti-jump penalty: if the track center is far from the current cursor,
    // it's likely a false detection. Use a progressive penalty starting at
    // 0.5x the track's diagonal (~90px for typical shapes). Normal movement
    // stays within this; jumps of 100px+ get penalized.
    let track_center = mid_point(track.rect());
    let dx = (track_center.x - last_cursor.x) as f64;
    let dy = (track_center.y - last_cursor.y) as f64;
    let distance = (dx * dx + dy * dy).sqrt();
    let track_diag = ((track.rect().width.pow(2) + track.rect().height.pow(2)) as f64).sqrt();
    let ratio = distance / track_diag.max(1.0);
    // Progressive: 0.5x = no penalty, 2.0x = 0.01x (near-zero)
    let jump_penalty = if ratio <= 0.5 {
        1.0
    } else if ratio >= 2.0 {
        0.01
    } else {
        // Linear interpolation between 0.5 and 2.0
        1.0 - 0.99 * ((ratio - 0.5) / 1.5)
    };

    let final_score = score * distance_penalty * jump_penalty;
    if final_score < 0.01 {
        return None;
    }

    Some(final_score)
}

fn track_background_degree(track: &STrack, bg_direction: Point2d) -> Option<f64> {
    let dir = unit(track.kalman_velocity())?;
    let dot = dir.dot(bg_direction);
    let det = dir.cross(bg_direction);
    Some(det.atan2(dot).to_degrees().abs())
}

fn estimate_background_direction(last_cursor: Option<Point>, tracks: &[STrack]) -> Option<Point2d> {
    let mut last_rect_contains_cursor = None;
    let filtered = tracks
        .iter()
        .filter(|track| {
            if track.tracklet_len() < 5 {
                return false;
            }

            if last_rect_contains_cursor.is_some_and(|rect: Rect| (rect & track.rect()).area() > 0)
            {
                return false;
            }

            let Some(last_cursor) = last_cursor else {
                return true;
            };

            let rect = track.rect();
            if rect.contains(last_cursor) {
                if last_rect_contains_cursor.is_none() {
                    last_rect_contains_cursor = Some(rect);
                }

                return false;
            }

            let norm = (mid_point(track.rect()) - last_cursor).norm();
            norm >= diag(track.rect())
        })
        .map(STrack::kalman_velocity)
        .collect::<Vec<Point2d>>();
    if filtered.len() < 3 {
        return None;
    }

    let velocity_sum = filtered
        .into_iter()
        .fold(Point2d::default(), |acc, v| acc + v);
    let velocity_unit = unit(velocity_sum)?;

    Some(velocity_unit)
}

fn diag(rect: Rect) -> f64 {
    ((rect.width.pow(2) + rect.height.pow(2)) as f64).sqrt()
}

fn unit<T>(point: Point_<T>) -> Option<Point_<T>>
where
    T: Copy,
    Point_<T>: Div<f64, Output = Point_<T>>,
    f64: From<T>,
{
    let norm = point.norm();
    if norm < 1e-3 {
        return None;
    }

    Some(point / norm)
}

/// Compute the principal-axis orientation angle of `roi` using image
/// moments. Returns the angle in radians in the range [-π/2, π/2].
///
/// The angle is computed from the central moments μ₁₁, μ₂₀, μ₀₂:
///
///   θ = 0.5 · atan2(2·μ₁₁, μ₂₀ − μ₀₂)
///
/// A self-rotating shape will have a continuously drifting θ; a
/// translating shape will have a nearly constant θ.
fn compute_principal_axis_angle(
    roi: &(impl MatTraitConst + ToInputArray),
) -> Option<f64> {
    let mut gray = Mat::default();
    // Convert to grayscale — moments require a single-channel image.
    imgproc::cvt_color_def(roi, &mut gray, imgproc::COLOR_BGR2GRAY).ok()?;
    let moments = imgproc::moments(&gray, false).ok()?;
    let denom = moments.mu20 - moments.mu02;
    if denom.abs() < 1e-10 {
        return None; // axis-aligned or degenerate
    }
    Some(0.5 * (2.0 * moments.mu11).atan2(denom))
}
