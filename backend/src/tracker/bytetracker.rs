use std::mem;

use crate::tracker::{
    Detection,
    strack::{STrack, TrackState},
    tlwh_to_xyah,
};

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum IouGating {
    None,
    Position,
    Full,
}

/// An extended [BYTETracker] implementation by GPT-5.
///
/// [BYTETracker]: https://github.com/ultralytics/ultralytics/blob/004d9730060e560c86ad79aaa1ab97167443be25/ultralytics/trackers/byte_tracker.py#L231
#[derive(Debug)]
pub struct ByteTracker {
    initialized: bool,
    tracked: Vec<STrack>,
    unconfirmed: Vec<STrack>,
    lost: Vec<STrack>,
    frame_id: u64,
    max_time_lost: u64,
    high_match_score_threshold: f32,
    low_match_score_threshold: f32,
    new_track_score_threshold: f32,
    gating: IouGating,
}

impl ByteTracker {
    pub fn new(
        max_time_lost: u64,
        high_match_score_threshold: f32,
        low_match_score_threshold: f32,
        new_track_score_threshold: f32,
        gating: IouGating,
    ) -> Self {
        Self {
            initialized: false,
            tracked: vec![],
            unconfirmed: vec![],
            lost: vec![],
            frame_id: 0,
            max_time_lost,
            high_match_score_threshold,
            low_match_score_threshold,
            new_track_score_threshold,
            gating,
        }
    }

    pub fn update(&mut self, detections: Vec<Detection>) -> Vec<STrack> {
        self.frame_id += 1;
        let total_detections = detections.len();

        self.predict();

        let (low_detection_tracks, high_detection_tracks): (Vec<STrack>, Vec<STrack>) = detections
            .into_iter()
            .map(|detection| STrack::new(detection.bbox, detection.score))
            .inspect(|t| {
                if t.score > 0.5 {
                    log::debug!(
                        "[byte_tracker] detection score={:.4} bbox=({},{},{},{})",
                        t.score,
                        t.tlwh[0] as i32,
                        t.tlwh[1] as i32,
                        t.tlwh[2] as i32,
                        t.tlwh[3] as i32,
                    );
                }
            })
            .filter(|track| track.score > self.low_match_score_threshold)
            .partition(|track| {
                self.low_match_score_threshold < track.score
                    && track.score < self.high_match_score_threshold
            });
        log::debug!(
            "[byte_tracker] frame={} total={} after_filter=({} low + {} high) initialized={} thresh=({},{},{})",
            self.frame_id,
            total_detections,
            low_detection_tracks.len(),
            high_detection_tracks.len(),
            self.initialized,
            self.low_match_score_threshold,
            self.high_match_score_threshold,
            self.new_track_score_threshold,
        );
        if self.init(&low_detection_tracks, &high_detection_tracks) {
            log::info!(
                "[byte_tracker] INIT frame={} activated {} tracks",
                self.frame_id,
                self.tracked.len()
            );
            return self.tracked.clone();
        }

        let (mut activated, unmatched_tracks, unmatched_detection_tracks) =
            self.associate_high_detections(high_detection_tracks);

        let lost =
            self.associate_low_detections(unmatched_tracks, low_detection_tracks, &mut activated);

        let unconfirmed = self.associate_unconfirmed(unmatched_detection_tracks, &mut activated);

        let (tracked, lost) = remove_duplicate_stracks(activated, lost, self.gating);
        self.tracked = tracked;
        self.lost = lost;
        self.unconfirmed = unconfirmed;

        self.tracked.clone()
    }

    fn predict(&mut self) {
        for track in &mut self.tracked {
            track.predict();
        }
        for track in &mut self.lost {
            track.predict();
        }
        for track in &mut self.unconfirmed {
            track.predict();
        }
    }

    fn init(&mut self, low_detection_tracks: &[STrack], high_detection_tracks: &[STrack]) -> bool {
        if self.initialized {
            return false;
        }

        let all_tracks: Vec<STrack> = low_detection_tracks
            .iter()
            .cloned()
            .chain(high_detection_tracks.iter().cloned())
            .map(|mut track| {
                track.activate(self.frame_id, track.score);
                track
            })
            .collect();

        // Don't initialize with zero tracks — wait for a frame with actual
        // detections. This prevents the tracker from getting stuck when the
        // first few frames have all-zero confidence scores (e.g. when the
        // video starts before the transparent shape puzzle phase).
        if all_tracks.is_empty() {
            return false;
        }

        self.initialized = true;
        self.tracked = all_tracks;
        true
    }

    fn associate_high_detections(
        &mut self,
        detection_tracks: Vec<STrack>,
    ) -> (Vec<STrack>, Vec<STrack>, Vec<STrack>) {
        let mut current_tracks = vec![];
        current_tracks.append(&mut self.tracked);
        current_tracks.append(&mut self.lost);

        let cost = iou_distance(&current_tracks, &detection_tracks, self.gating);
        let (matches, unmatched_tracks, unmatched_detections) = linear_assignment(cost, 0.5);

        let mut activated = vec![];
        for (ci, di) in matches {
            let mut track = current_tracks[ci].clone();
            let detection = &detection_tracks[di];

            match track.state {
                TrackState::Tracked => {
                    track.update(detection.tlwh, self.frame_id, detection.score);
                    activated.push(track);
                }
                TrackState::Lost => {
                    track.reactivate(detection.tlwh, self.frame_id, detection.score);
                    activated.push(track);
                }
            }
        }

        (
            activated,
            unmatched_tracks
                .into_iter()
                .map(|i| current_tracks[i].clone())
                .collect(),
            unmatched_detections
                .into_iter()
                .map(|i| detection_tracks[i].clone())
                .collect::<Vec<STrack>>(),
        )
    }

    fn associate_low_detections(
        &mut self,
        remain_tracks: Vec<STrack>,
        detection_tracks: Vec<STrack>,
        activated: &mut Vec<STrack>,
    ) -> Vec<STrack> {
        let cost = iou_distance(&remain_tracks, &detection_tracks, self.gating);
        let (matches, unmatched_tracks, _) = linear_assignment(cost, 0.5);

        let mut lost = vec![];
        for (ci, di) in matches {
            let mut track = remain_tracks[ci].clone();
            let detection = &detection_tracks[di];

            match track.state {
                TrackState::Tracked => {
                    track.update(detection.tlwh, self.frame_id, detection.score);
                    activated.push(track);
                }
                TrackState::Lost => {
                    track.reactivate(detection.tlwh, self.frame_id, detection.score);
                    activated.push(track);
                }
            }
        }

        for ci in unmatched_tracks {
            let mut track = remain_tracks[ci].clone();
            if self.frame_id - track.frame_id <= self.max_time_lost {
                track.mark_lost();
                lost.push(track);
            }
        }

        lost
    }

    fn associate_unconfirmed(
        &mut self,
        detection_tracks: Vec<STrack>,
        activated: &mut Vec<STrack>,
    ) -> Vec<STrack> {
        if self.unconfirmed.is_empty() {
            return detection_tracks
                .into_iter()
                .filter(|track| track.score >= self.new_track_score_threshold)
                .map(|mut track| {
                    track.activate(self.frame_id, track.score);
                    track
                })
                .collect();
        }

        let current_unconfirmed = mem::take(&mut self.unconfirmed);
        let cost = iou_distance(&current_unconfirmed, &detection_tracks, self.gating);
        let (matches, _, unmatched_detections) = linear_assignment(cost, 0.7);

        for (ui, di) in matches {
            let mut track = current_unconfirmed[ui].clone();
            let detection = &detection_tracks[di];

            track.update(detection.tlwh, self.frame_id, detection.score);
            activated.push(track);
        }

        unmatched_detections
            .into_iter()
            .filter_map(|di| {
                let mut track = detection_tracks[di].clone();
                if track.score < self.new_track_score_threshold {
                    return None;
                }

                track.activate(self.frame_id, track.score);
                Some(track)
            })
            .collect()
    }
}

fn iou_tlwh(a: [f32; 4], b: [f32; 4]) -> f32 {
    let ax1 = a[0];
    let ay1 = a[1];
    let ax2 = a[0] + a[2];
    let ay2 = a[1] + a[3];

    let bx1 = b[0];
    let by1 = b[1];
    let bx2 = b[0] + b[2];
    let by2 = b[1] + b[3];

    let inter_x1 = ax1.max(bx1);
    let inter_y1 = ay1.max(by1);
    let inter_x2 = ax2.min(bx2);
    let inter_y2 = ay2.min(by2);

    let inter_w = (inter_x2 - inter_x1).max(0.0);
    let inter_h = (inter_y2 - inter_y1).max(0.0);
    let inter_area = inter_w * inter_h;

    let area_a = a[2] * a[3];
    let area_b = b[2] * b[3];

    inter_area / (area_a + area_b - inter_area + 1e-6)
}

fn iou_distance(tracks: &[STrack], detections: &[STrack], gating: IouGating) -> Vec<Vec<f32>> {
    let mut cost = vec![vec![0.0; detections.len()]; tracks.len()];

    for (i, t) in tracks.iter().enumerate() {
        for (j, d) in detections.iter().enumerate() {
            match gating {
                IouGating::Position | IouGating::Full => {
                    let measurement = tlwh_to_xyah(d.tlwh);
                    let position_only = matches!(gating, IouGating::Position);

                    if t.kalman.gate(measurement, position_only) {
                        cost[i][j] = 1e6; // forbid
                    } else {
                        cost[i][j] = 1.0 - iou_tlwh(t.kalman_tlwh(), d.tlwh);
                    }
                }
                IouGating::None => {
                    cost[i][j] = 1.0 - iou_tlwh(t.kalman_tlwh(), d.tlwh);
                }
            }
        }
    }

    cost
}

fn linear_assignment(
    costs: Vec<Vec<f32>>,
    thresh: f32,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    use lapjv::{Matrix, lapjv};

    let n = costs.len();
    let m = if n > 0 { costs[0].len() } else { 0 };
    if n == 0 || m == 0 {
        return (vec![], (0..n).collect(), vec![]);
    }

    let k = n.max(m);
    let mut data = vec![1_000_000.0; k * k];
    for i in 0..n {
        for j in 0..m {
            data[i * k + j] = costs[i][j];
        }
    }

    let mat = Matrix::from_shape_vec((k, k), data).unwrap();
    let (x, _) = lapjv(&mat).expect("lapjv failed");

    let mut matches = Vec::new();
    let mut unmatched_a = Vec::new();
    let mut unmatched_b = vec![true; m];

    for i in 0..n {
        let j = x[i];
        if j < m && costs[i][j] <= thresh {
            matches.push((i, j));
            unmatched_b[j] = false;
        } else {
            unmatched_a.push(i);
        }
    }

    let unmatched_b: Vec<usize> = unmatched_b
        .iter()
        .enumerate()
        .filter_map(|(j, &u)| if u { Some(j) } else { None })
        .collect();

    (matches, unmatched_a, unmatched_b)
}

fn remove_duplicate_stracks(
    a: Vec<STrack>,
    b: Vec<STrack>,
    gating: IouGating,
) -> (Vec<STrack>, Vec<STrack>) {
    let cost = iou_distance(&a, &b, gating);

    let mut duplicate_a = vec![false; a.len()];
    let mut duplicate_b = vec![false; b.len()];

    for i in 0..a.len() {
        for j in 0..b.len() {
            if cost[i][j] < 0.15 {
                let time_a = a[i].frame_id - a[i].start_frame_id;
                let time_b = b[j].frame_id - b[j].start_frame_id;

                if time_a > time_b {
                    duplicate_b[j] = true;
                } else {
                    duplicate_a[i] = true;
                }
            }
        }
    }

    let filtered_a = a
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !duplicate_a[*i])
        .map(|(_, t)| t)
        .collect();

    let filtered_b = b
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !duplicate_b[*i])
        .map(|(_, t)| t)
        .collect();

    (filtered_a, filtered_b)
}
