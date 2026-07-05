use std::{
    collections::{HashMap, VecDeque},
    ops::Div,
    sync::atomic::{AtomicU64, Ordering},
};

use log::{debug, info};
use opencv::core::{Point, Point_, Point2d, Rect};

use crate::{
    detect::Detector,
    run::FPS,
    tracker::{ByteTracker, Detection, IouGating, STrack},
};

/// How aggressively the cursor moves toward the target each frame.
/// 0.35 = move 35% of the way each tick. Lower = smoother but more lag.
const CURSOR_SMOOTHING: f64 = 0.35;

/// Reduced smoothing factor used for the first few frames after switching
/// to a distant track, to avoid a visible "lurch."
const CURSOR_SMOOTHING_SLOW: f64 = 0.12;

/// Frames to use the slow smoothing factor after a distant track switch.
const CURSOR_SMOOTHING_SLOW_FRAMES: u32 = 10;

#[derive(Debug)]
pub struct TransparentShapeSolver {
    tracker: ByteTracker,
    current_track_id: Option<u64>,
    candidate_track_id: Option<u64>,
    candidate_track_count: u32,
    last_cursor: Option<Point>,
    last_velocity: Option<Point2d>,
    bg_direction: Point2d,
    smoothed_cursor: Option<Point2d>,
    /// How many consecutive frames the current track has been tracked.
    current_track_tenure: u32,
    /// Countdown of frames to use slow cursor smoothing after a track switch.
    slow_smoothing_remaining: u32,
    /// Per-track aspect-ratio history for rotation detection.
    /// The correct shape self-rotates, causing its aspect ratio to oscillate.
    /// Distractors only translate, so their aspect ratio stays stable.
    aspect_history: HashMap<u64, VecDeque<f64>>,
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
            smoothed_cursor: None,
            current_track_tenure: 0,
            slow_smoothing_remaining: 0,
            aspect_history: HashMap::new(),
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
            smoothed_cursor: None,
            current_track_tenure: 0,
            slow_smoothing_remaining: 0,
            aspect_history: HashMap::new(),
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

        // ── rotation detection via aspect-ratio oscillation ──
        // The correct shape self-rotates, causing its bounding-box
        // aspect ratio to oscillate. Distractors only translate, so
        // their aspect ratio stays nearly constant. We track the
        // width/height ratio per track over a sliding window and
        // compute the variance — the highest-variance track is the
        // rotating one.
        const ASPECT_WINDOW: usize = 20;
        const ROTATION_CHECK_INTERVAL: u64 = 30;

        for t in &tracks {
            let bbox = t.rect();
            let aspect = if bbox.height > 0 {
                bbox.width as f64 / bbox.height as f64
            } else {
                0.0
            };
            let history = self.aspect_history.entry(t.track_id()).or_default();
            history.push_back(aspect);
            if history.len() > ASPECT_WINDOW {
                history.pop_front();
            }
        }
        // Purge dead tracks from history.
        let live_ids: Vec<u64> = tracks.iter().map(|t| t.track_id()).collect();
        self.aspect_history.retain(|id, _| live_ids.contains(id));

        self.update_initial_track_if_needed(region, &tracks);
        self.update_background_direction(&tracks);

        // Periodically check whether we should switch to a track with
        // much stronger rotation signal.
        static SOLVE_COUNT: AtomicU64 = AtomicU64::new(0);
        let solve_n = SOLVE_COUNT.fetch_add(1, Ordering::Relaxed);
        if solve_n % ROTATION_CHECK_INTERVAL == 0
            && self.current_track_id.is_some()
        {
            let mut scores: Vec<(u64, f64)> = self
                .aspect_history
                .iter()
                .filter_map(|(id, history)| {
                    if history.len() < ASPECT_WINDOW / 2 {
                        return None; // not enough data yet
                    }
                    let mean: f64 = history.iter().sum::<f64>() / history.len() as f64;
                    let variance: f64 = history
                        .iter()
                        .map(|a| (a - mean).powi(2))
                        .sum::<f64>()
                        / history.len() as f64;
                    Some((*id, variance))
                })
                .collect();

            // Sort by variance descending — highest rotation first.
            scores.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap());

            if let Some(current_id) = self.current_track_id {
                let current_score = scores
                    .iter()
                    .find(|(id, _)| *id == current_id)
                    .map(|(_, s)| *s)
                    .unwrap_or(0.0);

                // Switch if another track has at least 3× higher
                // aspect-ratio variance AND we have enough tenure
                // that we're not just in the initial grace period.
                if let Some((best_id, best_score)) = scores.first() {
                    if *best_id != current_id
                        && *best_score > current_score * 3.0
                        && *best_score > 0.001
                        && self.current_track_tenure > ASPECT_WINDOW as u32
                    {
                        debug!(
                            target: "backend/player",
                            "shape id switches from {} to {} (rotation: {:.6} vs {:.6})",
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
                let next_cursor = predicted_center(track);
                if self.current_track_id != Some(track.track_id()) {
                    debug!(target: "backend/player", "shape id switches from {:?} to {}", self.current_track_id, track.track_id());
                }
                self.current_track_id = Some(track.track_id());
                self.last_cursor = Some(next_cursor);
                self.last_velocity = Some(track.kalman_velocity());

                #[cfg(debug_assertions)]
                if self.is_debugging {
                    debug_transparent_shapes(
                        detector,
                        &tracks,
                        region,
                        next_cursor,
                        self.bg_direction,
                    );
                }

                Some(self.smooth_cursor(region.tl() + next_cursor))
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

                #[cfg(debug_assertions)]
                if self.is_debugging {
                    debug_transparent_shapes(
                        detector,
                        &tracks,
                        region,
                        next_cursor,
                        self.bg_direction,
                    );
                }

                Some(self.smooth_cursor(absolute_next_cursor))
            }
        }
    }

    /// Applies exponential smoothing to the raw cursor position.
    ///
    /// Each frame, the cursor moves toward the raw target. Normally 35% per
    /// frame (~88% in 5 frames). After a distant track switch, uses 12% for
    /// 10 frames to avoid a visible lurch.
    fn smooth_cursor(&mut self, raw: Point) -> Point {
        let factor = if self.slow_smoothing_remaining > 0 {
            self.slow_smoothing_remaining -= 1;
            CURSOR_SMOOTHING_SLOW
        } else {
            CURSOR_SMOOTHING
        };
        let raw_f = Point2d::new(raw.x as f64, raw.y as f64);
        let smoothed = match self.smoothed_cursor {
            Some(prev) => prev + (raw_f - prev) * factor,
            None => raw_f,
        };
        self.smoothed_cursor = Some(smoothed);
        Point::new(smoothed.x.round() as i32, smoothed.y.round() as i32)
    }

    fn update_initial_track_if_needed(&mut self, region: Rect, tracks: &[STrack]) {
        if self.current_track_id.is_none() {
            let region_mid = mid_point(Rect::new(0, 0, region.width, region.height));
            if let Some(track) = find_track_closest_to(region_mid, tracks) {
                self.current_track_id = Some(track.track_id());
                self.last_cursor = Some(mid_point(track.rect()));
                self.last_velocity = Some(track.kalman_velocity());
            }
        }
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

fn predicted_center(track: &STrack) -> Point {
    let v = track.kalman_velocity();
    let point = mid_point(track.kalman_rect());

    Point::new(
        (point.x as f64 + v.x).round() as i32,
        (point.y as f64 + v.y).round() as i32,
    )
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
