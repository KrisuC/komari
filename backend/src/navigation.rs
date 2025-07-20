use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::{Debug, Formatter},
    rc::Rc,
    time::Instant,
};

use anyhow::{Result, anyhow};
use base64::{Engine, prelude::BASE64_STANDARD};
use log::{debug, info};
#[cfg(test)]
use mockall::automock;
use opencv::{
    core::{Mat, Rect, Vector},
    imgcodecs::{IMREAD_COLOR, IMREAD_GRAYSCALE, imdecode},
};

use crate::{
    ActionKeyDirection, ActionKeyWith, KeyBinding, Position,
    context::Context,
    database::{NavigationPath, NavigationTransition, query_navigation_paths},
    detect::Detector,
    minimap::Minimap,
    player::{PlayerAction, PlayerActionKey, PlayerState},
};

/// Internal representation of [`NavigationPath`].
///
/// This is used for eagerly resolving all of a path's referenced ids.
#[derive(Clone)]
struct Path {
    id: i64,
    minimap_snapshot_base64: String,
    name_snapshot_base64: String,
    points: Vec<Point>,
}

impl Debug for Path {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Path")
            .field("minimap_snapshot_base64", &"...")
            .field("name_snapshot_base64", &"...")
            .field("points", &self.points)
            .finish()
    }
}

/// Internal representation of [`NavigationPoint`].
#[derive(Debug, Clone)]
struct Point {
    next_path: Option<Rc<RefCell<Path>>>, // TODO: How to Rc<RefCell<Path>> into Rc<Path>?
    x: i32,
    y: i32,
    transition: NavigationTransition,
}

/// Next point computation state to navigate the player to [`Navigator::destination_path_id`].
#[derive(Debug, Clone)]
enum PointState {
    Dirty,
    Completed,
    Unreachable,
    Next(i32, i32, NavigationTransition, Option<Rc<RefCell<Path>>>),
}

/// Update state when [`Navigator::path_dirty`] is `true`.
#[derive(Debug)]
enum UpdateState {
    Pending,
    Completed,
    NoMatch,
}

/// A data source to query [`NavigationPath`].
///
/// This helps abstracting out database and useful for tests.
#[cfg_attr(test, automock)]
pub trait NavigatorDataSource: Debug + 'static {
    fn query_paths(&self) -> Result<Vec<NavigationPath>>;
}

#[derive(Debug)]
struct DefaultNavigatorDataSource;

impl NavigatorDataSource for DefaultNavigatorDataSource {
    fn query_paths(&self) -> Result<Vec<NavigationPath>> {
        query_navigation_paths()
    }
}

/// Manages navigation paths to reach a certain minimap.
#[derive(Debug)]
pub struct Navigator {
    // TODO: Cache mat?
    /// Data source for querying [`NavigationPath`]s.
    source: Box<dyn NavigatorDataSource>,
    /// Base path to search for navigation points.
    base_path: Option<Rc<RefCell<Path>>>,
    /// The player's current path.
    current_path: Option<Rc<RefCell<Path>>>,
    /// Whether paths are dirty.
    ///
    /// If true, [`Self::base_path`] and [`Self::current_path`] must be updated before computing
    /// the next navigation point to reach [`Self::destination_path_id`].
    path_dirty: bool,
    /// Number of times to retry updating when paths are dirty.
    path_dirty_retry_count: u32,
    /// Last time an update attempt was made.
    path_last_update: Instant,
    /// Cached next point navigation computation.
    last_point_state: Option<PointState>,
    destination_path_id: Option<i64>,
}

impl Default for Navigator {
    fn default() -> Self {
        Self::new(DefaultNavigatorDataSource)
    }
}

impl Navigator {
    fn new(source: impl NavigatorDataSource) -> Self {
        Self {
            source: Box::new(source),
            base_path: None,
            current_path: None,
            path_dirty: true,
            path_dirty_retry_count: 0,
            path_last_update: Instant::now(),
            last_point_state: None,
            destination_path_id: None,
        }
    }

    /// Navigates the player to the currently set [`Self::destination_path_id`].
    ///
    /// Returns `true` if the player has reached the destination.
    pub fn navigate_player(&mut self, context: &Context, player: &mut PlayerState) -> bool {
        if context.operation.halting() {
            return false;
        }

        self.last_point_state = Some(self.compute_next_point());
        match self.last_point_state.as_ref().expect("has value") {
            PointState::Dirty => {
                if context.did_minimap_changed {
                    player.take_priority_action();
                }
                false
            }
            PointState::Completed | PointState::Unreachable => true,
            PointState::Next(x, y, transition, _) => {
                match transition {
                    NavigationTransition::Portal => {
                        if !player.has_priority_action() {
                            let position = Position {
                                x: *x,
                                y: *y,
                                x_random_range: 0,
                                allow_adjusting: true,
                            };
                            let key = PlayerActionKey {
                                key: KeyBinding::Up,
                                link_key: None,
                                count: 1,
                                position: Some(position),
                                direction: ActionKeyDirection::Any,
                                with: ActionKeyWith::Stationary,
                                wait_before_use_ticks: 5,
                                wait_before_use_ticks_random_range: 0,
                                wait_after_use_ticks: 0,
                                wait_after_use_ticks_random_range: 0,
                            };
                            player.set_priority_action(None, PlayerAction::Key(key));
                        }
                    }
                }

                false
            }
        }
    }

    /// Whether the last point to navigate to was available or the navigation is completed.
    #[inline]
    pub fn was_last_point_available_or_completed(&self) -> bool {
        matches!(
            self.last_point_state,
            Some(PointState::Next(_, _, _, _) | PointState::Completed)
        )
    }

    fn compute_next_point(&self) -> PointState {
        fn search_point(from: Rc<RefCell<Path>>, to_id: i64) -> Option<Point> {
            let from_id = from.borrow().id;
            let mut visiting_paths = vec![(from, None, None)];
            let mut came_from = HashMap::<i64, (Option<Rc<RefCell<Path>>>, Option<Point>)>::new();

            while let Some((path, from_path, from_point)) = visiting_paths.pop() {
                let path_borrow = path.borrow();
                if came_from
                    .try_insert(path_borrow.id, (from_path, from_point))
                    .is_err()
                {
                    continue;
                }

                // Trace back to start_path to find the first point to move
                if path_borrow.id == to_id {
                    let mut current = path_borrow.id;
                    while let Some((Some(from_path), Some(from_point))) = came_from.get(&current) {
                        if from_path.borrow().id == from_id {
                            return Some(from_point.clone());
                        }
                        current = from_path.borrow().id;
                    }
                }

                for point in path_borrow.points.iter() {
                    if let Some(next_path) = point.next_path.clone() {
                        visiting_paths.push((next_path, Some(path.clone()), Some(point.clone())));
                    }
                }
            }

            None
        }

        if self.path_dirty {
            return PointState::Dirty;
        }
        // Re-use cached point
        if matches!(
            self.last_point_state,
            Some(PointState::Next(_, _, _, _) | PointState::Completed | PointState::Unreachable)
        ) {
            return self.last_point_state.clone().expect("has value");
        }
        if self.destination_path_id.is_none() {
            return PointState::Completed;
        }

        let path_id = self.destination_path_id.expect("has value");
        if self
            .current_path
            .as_ref()
            .is_some_and(|path| path.borrow().id == path_id)
        {
            return PointState::Completed;
        }

        // Search from current forward
        if let Some(point) = self
            .current_path
            .clone()
            .and_then(|path| search_point(path, path_id))
        {
            return PointState::Next(point.x, point.y, point.transition, point.next_path.clone());
        }

        PointState::Unreachable
    }

    #[inline]
    pub fn mark_dirty(&mut self) {
        // Do not reset `base_path` and `current_path` here so that
        // `update_current_path_from_current_location` will try to reuse that when looking up.
        self.path_dirty = true;
        self.path_dirty_retry_count = 0;
        self.last_point_state = None;
    }

    #[inline]
    pub fn mark_dirty_with_destination(&mut self, path_id: Option<i64>) {
        self.destination_path_id = path_id;
        self.mark_dirty();
    }

    #[inline]
    pub fn update(&mut self, context: &Context) {
        const UPDATE_RETRY_MAX_COUNT: u32 = 3;

        if context.did_minimap_changed {
            self.mark_dirty();
        }
        if self.path_dirty {
            match self.update_current_path_from_current_location(context) {
                UpdateState::Pending => (),
                UpdateState::Completed => self.path_dirty = false,
                UpdateState::NoMatch => {
                    if self.path_dirty_retry_count < UPDATE_RETRY_MAX_COUNT {
                        self.path_dirty_retry_count += 1;
                    } else {
                        self.path_dirty = false;
                    }
                }
            }
        }
    }

    // TODO: Do this on background thread?
    fn update_current_path_from_current_location(&mut self, context: &Context) -> UpdateState {
        const UPDATE_INTERVAL_SECS: u64 = 2;

        let minimap_bbox = match context.minimap {
            Minimap::Idle(idle) => idle.bbox,
            Minimap::Detecting => return UpdateState::Pending,
        };
        let instant = Instant::now();
        if instant.duration_since(self.path_last_update).as_secs() < UPDATE_INTERVAL_SECS {
            return UpdateState::Pending;
        }
        self.path_last_update = instant;
        debug!(target: "navigator", "updating current path from current location...");

        let detector = context
            .detector
            .as_ref()
            .expect("detector must available because minimap is idle")
            .as_ref();
        let Ok(minimap_name_bbox) = detector.detect_minimap_name(minimap_bbox) else {
            return UpdateState::NoMatch;
        };

        // Try from next_path if previously exists due to player navigating
        if let Some(PointState::Next(_, _, _, Some(next_path))) = self.last_point_state.take()
            && let Ok(current_path) =
                find_current_from_base_path(next_path, detector, minimap_bbox, minimap_name_bbox)
        {
            info!(target: "navigator", "current path updated from previous point's next path");
            self.current_path = Some(current_path);
            return UpdateState::Completed;
        }

        // Try from base_path if previously exists
        if let Some(base_path) = self.base_path.clone() {
            if let Ok(current_path) =
                find_current_from_base_path(base_path, detector, minimap_bbox, minimap_name_bbox)
            {
                info!(target: "navigator", "current path updated from previous base path");
                self.current_path = Some(current_path);
                return UpdateState::Completed;
            } else {
                self.base_path = None;
                self.current_path = None;
            }
        }

        // Query from database
        let paths = self
            .source
            .query_paths()
            .unwrap_or_default()
            .into_iter()
            .map(|path| (path.id.expect("valid id"), path))
            .collect::<HashMap<_, _>>();
        let mut visited_ids = HashSet::new();

        for path_id in paths.keys() {
            if !visited_ids.insert(*path_id) {
                continue;
            }
            let Ok((base_path, visited)) = build_base_path_from(&paths, *path_id) else {
                continue;
            };
            visited_ids.extend(visited);

            let Ok(current_path) = find_current_from_base_path(
                base_path.clone(),
                detector,
                minimap_bbox,
                minimap_name_bbox,
            ) else {
                continue;
            };
            info!(target: "navigator", "current path updated from database");

            self.base_path = Some(base_path);
            self.current_path = Some(current_path);
            return UpdateState::Completed;
        }

        UpdateState::NoMatch
    }
}

fn build_base_path_from(
    paths: &HashMap<i64, NavigationPath>,
    path_id: i64,
) -> Result<(Rc<RefCell<Path>>, HashSet<i64>)> {
    let mut visiting_paths = HashMap::new();
    let mut visited_path_ids = HashSet::new();
    let mut visiting_path_ids = vec![path_id];

    while let Some(path_id) = visiting_path_ids.pop() {
        if !visited_path_ids.insert(path_id) {
            continue;
        }

        let path = paths.get(&path_id).expect("exists");
        let inner_path = visiting_paths
            .entry(path_id)
            .or_insert_with(|| {
                Rc::new(RefCell::new(Path {
                    id: path_id,
                    minimap_snapshot_base64: path.minimap_snapshot_base64.clone(),
                    name_snapshot_base64: path.name_snapshot_base64.clone(),
                    points: vec![],
                }))
            })
            .clone();

        for point in path.points.iter().copied() {
            let next_path = point
                .next_path_id
                .as_ref()
                .and_then(|path_id| visiting_paths.get(path_id).cloned())
                .or_else(|| {
                    let path_id = point.next_path_id?;
                    let path = paths.get(&path_id).expect("exists");
                    let inner_path = Rc::new(RefCell::new(Path {
                        id: path_id,
                        minimap_snapshot_base64: path.minimap_snapshot_base64.clone(),
                        name_snapshot_base64: path.name_snapshot_base64.clone(),
                        points: vec![],
                    }));

                    visiting_paths.insert(path_id, inner_path.clone());
                    Some(inner_path)
                });

            inner_path.borrow_mut().points.push(Point {
                next_path,
                x: point.x,
                y: point.y,
                transition: point.transition,
            });

            if let Some(id) = point.next_path_id {
                visiting_path_ids.push(id);
            }
        }
    }

    Ok((
        visiting_paths.remove(&path_id).expect("root path exists"),
        visited_path_ids,
    ))
}

fn find_current_from_base_path(
    base_path: Rc<RefCell<Path>>,
    detector: &dyn Detector,
    minimap_bbox: Rect,
    minimap_name_bbox: Rect,
) -> Result<Rc<RefCell<Path>>> {
    let mut visited_ids = HashSet::new();
    let mut visiting_paths = vec![base_path];
    let mut matches = vec![];

    while let Some(path) = visiting_paths.pop() {
        let path_borrow = path.borrow();
        if !visited_ids.insert(path_borrow.id) {
            continue;
        }
        for point in &path_borrow.points {
            if let Some(path) = point.next_path.clone() {
                visiting_paths.push(path);
            }
        }

        let name_mat = decode_base64_to_mat(&path_borrow.name_snapshot_base64, true)?;
        let minimap_mat = decode_base64_to_mat(&path_borrow.minimap_snapshot_base64, false)?;
        if let Ok(score) =
            detector.detect_minimap_match(&minimap_mat, &name_mat, minimap_bbox, minimap_name_bbox)
        {
            debug!(target: "navigator", "candidate path found with score {score}");
            matches.push((score, path.clone()));
        }
    }

    matches
        .into_iter()
        .max_by(|(first_score, _), (second_score, _)| first_score.total_cmp(second_score))
        .map(|(_, path)| path)
        .ok_or(anyhow!("unable to determine current path"))
}

fn decode_base64_to_mat(base64: &str, grayscale: bool) -> Result<Mat> {
    let flag = if grayscale {
        IMREAD_GRAYSCALE
    } else {
        IMREAD_COLOR
    };
    let name_bytes = BASE64_STANDARD.decode(base64)?;
    let name_bytes = Vector::<u8>::from_iter(name_bytes);

    Ok(imdecode(&name_bytes, flag)?)
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use super::*;
    use crate::{database::NavigationPoint, detect::MockDetector, minimap::MinimapIdle};

    fn mock_navigation_path(id: Option<i64>, points: Vec<NavigationPoint>) -> NavigationPath {
        NavigationPath {
            id,
            minimap_snapshot_base64: "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAb0lEQVR4nGKZpBfKAANX6s3hbO6+y3D2GsV5cDYTA4mA9hoYDx3LgHP4LynD2UckjOHsp3c/0NFJJGtg2eR5B865XhcBZ7deQMRP0Y0ndHQS6fGgxGsL5+xSXAxnv+tYBGfnBryjo5NI1gAIAAD//9O1GVeWUw0pAAAAAElFTkSuQmCC".to_string(),
            name_snapshot_base64: "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAb0lEQVR4nGKZpBfKAANX6s3hbO6+y3D2GsV5cDYTA4mA9hoYDx3LgHP4LynD2UckjOHsp3c/0NFJJGtg2eR5B865XhcBZ7deQMRP0Y0ndHQS6fGgxGsL5+xSXAxnv+tYBGfnBryjo5NI1gAIAAD//9O1GVeWUw0pAAAAAElFTkSuQmCC".to_string(),
            name_snapshot_width: 2,
            name_snapshot_height: 5,
            points,
        }
    }

    #[test]
    fn build_base_path_from_valid_navigation_tree() {
        let path_d_id = 4;
        let path_d = mock_navigation_path(Some(path_d_id), vec![]);

        let path_e_id = 5;
        let path_e = mock_navigation_path(Some(path_e_id), vec![]);

        // Path C → E
        let path_c_id = 3;
        let path_c = mock_navigation_path(
            Some(path_c_id),
            vec![NavigationPoint {
                next_path_id: Some(path_e_id),
                x: 30,
                y: 30,
                transition: NavigationTransition::Portal,
            }],
        );

        let path_a_id = 1;
        // Path B → A, C
        let path_b_id = 2;
        let path_b = mock_navigation_path(
            Some(path_b_id),
            vec![
                NavigationPoint {
                    next_path_id: Some(path_c_id),
                    x: 20,
                    y: 20,
                    transition: NavigationTransition::Portal,
                },
                NavigationPoint {
                    next_path_id: Some(path_a_id),
                    x: 10,
                    y: 10,
                    transition: NavigationTransition::Portal,
                },
            ],
        );

        // Path A → B, D
        let path_a = mock_navigation_path(
            Some(path_a_id),
            vec![
                NavigationPoint {
                    next_path_id: Some(path_d_id),
                    x: 11,
                    y: 10,
                    transition: NavigationTransition::Portal,
                },
                NavigationPoint {
                    next_path_id: Some(path_b_id),
                    x: 10,
                    y: 10,
                    transition: NavigationTransition::Portal,
                },
            ],
        );

        let paths = HashMap::from_iter([
            (path_a_id, path_a.clone()),
            (path_b_id, path_b.clone()),
            (path_c_id, path_c.clone()),
            (path_d_id, path_d.clone()),
            (path_e_id, path_e.clone()),
        ]);

        // Check structure
        let (path, _) = build_base_path_from(&paths, path_a_id).expect("success");
        let path = path.borrow();
        assert_eq!(path.points.len(), 2);

        // Path D
        assert_eq!(path.points[0].x, 11);
        assert_eq!(path.points[0].y, 10);
        assert_eq!(path.points[0].transition, NavigationTransition::Portal);

        // Path B
        assert_eq!(path.points[1].x, 10);
        assert_eq!(path.points[1].y, 10);
        assert_eq!(path.points[1].transition, NavigationTransition::Portal);

        let d_path = path.points[0]
            .next_path
            .as_ref()
            .expect("Path D should exist");
        assert!(d_path.borrow().points.is_empty());

        let b_path = path.points[1]
            .next_path
            .as_ref()
            .expect("Path B should exist")
            .borrow();
        assert_eq!(b_path.points.len(), 2);
        assert_eq!(b_path.points[0].x, 20);
        assert_eq!(b_path.points[0].y, 20);
        assert_eq!(b_path.points[0].transition, NavigationTransition::Portal);

        // Path A in B
        assert_eq!(b_path.points[1].x, 10);
        assert_eq!(b_path.points[1].y, 10);
        assert_eq!(b_path.points[1].transition, NavigationTransition::Portal);

        let c_path = b_path.points[0]
            .next_path
            .as_ref()
            .expect("Path C should exist")
            .borrow();
        assert_eq!(c_path.points.len(), 1);

        // Path E
        assert_eq!(c_path.points[0].x, 30);
        assert_eq!(c_path.points[0].y, 30);
        assert_eq!(c_path.points[0].transition, NavigationTransition::Portal);

        let e_path = c_path.points[0]
            .next_path
            .as_ref()
            .expect("Path E should exist");
        assert!(e_path.borrow().points.is_empty());
    }

    #[test]
    fn compute_next_point_when_path_dirty() {
        let navigator = Navigator::default();

        let result = navigator.compute_next_point();

        assert!(matches!(result, PointState::Dirty));
    }

    #[test]
    fn compute_next_point_when_no_destination_path() {
        let navigator = Navigator {
            path_dirty: false,
            ..Default::default()
        };

        let result = navigator.compute_next_point();

        assert!(matches!(result, PointState::Completed));
    }

    #[test]
    fn compute_next_point_when_current_path_matches_destination() {
        let mut navigator = Navigator::default();
        let path = Path {
            id: 42,
            minimap_snapshot_base64: "".into(),
            name_snapshot_base64: "".into(),
            points: vec![],
        };
        navigator.current_path = Some(Rc::new(RefCell::new(path.clone())));
        navigator.destination_path_id = Some(42);
        navigator.path_dirty = false;

        let result = navigator.compute_next_point();

        assert!(matches!(result, PointState::Completed));
    }

    #[test]
    fn compute_next_point_returns_next_point_from_current_path() {
        let mut navigator = Navigator::default();
        let target_path = Path {
            id: 2,
            minimap_snapshot_base64: "".into(),
            name_snapshot_base64: "".into(),
            points: vec![],
        };
        let point = Point {
            x: 100,
            y: 200,
            transition: NavigationTransition::Portal,
            next_path: Some(Rc::new(RefCell::new(target_path.clone()))),
        };
        let path = Path {
            id: 1,
            minimap_snapshot_base64: "".into(),
            name_snapshot_base64: "".into(),
            points: vec![point.clone()],
        };
        navigator.current_path = Some(Rc::new(RefCell::new(path.clone())));
        navigator.destination_path_id = Some(2);
        navigator.path_dirty = false;

        let result = navigator.compute_next_point();

        match result {
            PointState::Next(x, y, transition, Some(next_path)) => {
                assert_eq!(x, 100);
                assert_eq!(y, 200);
                assert_eq!(transition, NavigationTransition::Portal);
                assert_eq!(next_path.borrow().id, 2);
            }
            _ => panic!("Unexpected PointState: {result:?}"),
        }
    }

    #[test]
    fn compute_next_point_unreachable_when_not_in_any_path() {
        let mut navigator = Navigator::default();
        let unrelated_path = Rc::new(RefCell::new(Path {
            id: 1,
            minimap_snapshot_base64: "".into(),
            name_snapshot_base64: "".into(),
            points: vec![],
        }));
        navigator.current_path = Some(unrelated_path.clone());
        navigator.base_path = Some(unrelated_path);
        navigator.destination_path_id = Some(42); // Not present
        navigator.path_dirty = false;

        let result = navigator.compute_next_point();

        assert!(matches!(result, PointState::Unreachable));
    }

    #[test]
    fn update_current_path_from_current_location_success() {
        let minimap_bbox = Rect::new(0, 0, 10, 10);
        let minimap_name_bbox = Rect::new(1, 1, 5, 5);
        let mut mock_detector = MockDetector::new();
        mock_detector
            .expect_detect_minimap_name()
            .returning(move |_| Ok(minimap_name_bbox));
        mock_detector
            .expect_detect_minimap_match()
            .returning(|_, _, _, _| Ok(0.75)); // Simulate successful match

        let mut minimap = MinimapIdle::default();
        minimap.bbox = minimap_bbox;
        let mut context = Context::new(None, Some(mock_detector));
        context.minimap = Minimap::Idle(minimap);

        let point = NavigationPoint {
            next_path_id: None,
            x: 5,
            y: 5,
            transition: NavigationTransition::Portal,
        };

        let mock_path = mock_navigation_path(Some(1), vec![point]);

        let mut mock_source = MockNavigatorDataSource::new();
        mock_source
            .expect_query_paths()
            .returning(move || Ok(vec![mock_path.clone()]));

        let mut navigator = Navigator::new(mock_source);

        // Force update
        navigator.path_last_update = Instant::now() - std::time::Duration::from_secs(10);

        let result = navigator.update_current_path_from_current_location(&context);

        assert_matches!(result, UpdateState::Completed);
        assert!(navigator.current_path.is_some());
        assert!(navigator.base_path.is_some());
    }
}
