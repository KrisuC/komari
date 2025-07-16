use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::{Debug, Formatter},
    rc::Rc,
    time::Instant,
};

use anyhow::{Result, anyhow, bail};
use base64::{Engine, prelude::BASE64_STANDARD};
use log::{debug, info};
#[cfg(test)]
use mockall::automock;
use opencv::{
    core::{Mat, Rect, Vector},
    imgcodecs::{IMREAD_COLOR, IMREAD_GRAYSCALE, imdecode},
};

use crate::{
    context::Context,
    database::{
        NavigationPath, NavigationTransition, query_navigation_path, query_navigation_paths,
    },
    detect::Detector,
    minimap::Minimap,
};

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

#[derive(Debug, Clone)]
struct Point {
    next_path: Option<Path>,
    x: i32,
    y: i32,
    transition: NavigationTransition,
}

#[derive(Debug, Clone, Copy)]
pub enum PointState {
    Dirty,
    Completed,
    Unreachable,
    Next((i32, i32, NavigationTransition)),
}

/// A data source to query [`NavigationPath`].
///
/// This helps abstracting out database and useful for tests.
#[cfg_attr(test, automock)]
pub trait NavigatorDataSource: Debug + 'static {
    fn query_path(&self, id: i64) -> Result<NavigationPath>;

    fn query_paths(&self) -> Result<Vec<NavigationPath>>;
}

#[derive(Debug)]
pub struct DefaultNavigatorDataSource;

impl NavigatorDataSource for DefaultNavigatorDataSource {
    fn query_path(&self, id: i64) -> Result<NavigationPath> {
        query_navigation_path(id)
    }

    fn query_paths(&self) -> Result<Vec<NavigationPath>> {
        query_navigation_paths()
    }
}

/// Manages navigation paths to reach a certain minimap.
#[derive(Debug)]
pub struct Navigator {
    // TODO: Cache mat?
    source: Box<dyn NavigatorDataSource>,
    base_path: Option<Path>,
    current_path: Option<Path>,
    path_dirty: bool,
    path_last_update: Instant,
    last_computed_point_state: Option<PointState>,
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
            path_last_update: Instant::now(),
            last_computed_point_state: None,
        }
    }

    #[inline]
    fn mark_path_dirty(&mut self) {
        self.path_dirty = true;
        self.last_computed_point_state = None;
    }

    pub fn compute_next_point_to_reach(&self, path_id: Option<i64>) -> PointState {
        if let Some(state) = self.last_computed_point_state {
            return state;
        }
        if self.path_dirty {
            return PointState::Dirty;
        }
        if path_id.is_none()
            || self
                .current_path
                .as_ref()
                .is_some_and(|path| path.id == path_id.expect("has value"))
        {
            return PointState::Completed;
        }

        // TODO: Reuse visit pattern
        let path_id = path_id.expect("has value");
        let start_path = self.current_path.as_ref().expect("not dirty");
        let mut visiting_paths = vec![(start_path, None, None)];
        let mut came_from = HashMap::<i64, (Option<&Path>, Option<&Point>)>::new();

        while let Some((path, from_path, from_point)) = visiting_paths.pop() {
            came_from.insert(path.id, (from_path, from_point));

            // Trace back to start_path to find the first point to move
            if path.id == path_id {
                let mut current = path.id;
                while let Some((Some(from_path), Some(from_point))) = came_from.get(&current) {
                    if from_path.id == start_path.id {
                        return PointState::Next((
                            from_point.x,
                            from_point.y,
                            from_point.transition,
                        ));
                    }
                    current = from_path.id;
                }
            }

            for point in &path.points {
                if let Some(next_path) = point.next_path.as_ref() {
                    visiting_paths.push((next_path, Some(path), Some(point)));
                }
            }
        }

        PointState::Unreachable
    }

    pub fn update(&mut self, context: &Context) {
        if context.did_minimap_changed {
            self.mark_path_dirty();
        }
        if self.path_dirty {
            self.path_dirty = self
                .update_current_path_from_current_location(context)
                .is_err();
        }
    }

    fn update_current_path_from_current_location(&mut self, context: &Context) -> Result<()> {
        const UPDATE_INTERVAL_SECS: u64 = 5;

        let instant = Instant::now();
        if instant.duration_since(self.path_last_update).as_secs() < UPDATE_INTERVAL_SECS {
            bail!("update debounce");
        }
        self.path_last_update = instant;
        debug!(target: "navigator", "updating current path from current location...");

        let detector = context
            .detector
            .as_ref()
            .ok_or(anyhow!("detector not available"))?
            .as_ref();
        let minimap_bbox = match context.minimap {
            Minimap::Idle(idle) => idle.bbox,
            Minimap::Detecting => bail!("minimap not idle"),
        };
        let minimap_name_bbox = detector.detect_minimap_name(minimap_bbox)?;
        // Try from base_path if previously exists
        if let Some(base_path) = self.base_path.as_ref() {
            if let Ok(current_path) =
                find_current_from_base_path(base_path, detector, minimap_bbox, minimap_name_bbox)
            {
                info!(target: "navigator", "current path updated from previous base path {current_path:?}");
                self.current_path = Some(current_path);
                return Ok(());
            } else {
                self.base_path = None;
                self.current_path = None;
            }
        }

        // Query from database
        let paths = find_root_paths(self.source.query_paths()?);
        for path in paths {
            let Ok(base_path) = build_base_path_from(self.source.as_ref(), path) else {
                continue;
            };
            let Ok(current_path) =
                find_current_from_base_path(&base_path, detector, minimap_bbox, minimap_name_bbox)
            else {
                continue;
            };
            info!(target: "navigator", "current path updated {current_path:?}");

            self.base_path = Some(base_path);
            self.current_path = Some(current_path);
            return Ok(());
        }

        bail!("unable to determine current location")
    }
}

fn build_base_path_from(source: &dyn NavigatorDataSource, path: NavigationPath) -> Result<Path> {
    #[derive(Debug)]
    struct VisitingPath {
        inner: Option<NavigationPath>,
        inner_associated_point: Option<VisitingPoint>,
        inner_children_points: Vec<Point>,
        parent: Option<Rc<RefCell<VisitingPath>>>,
    }

    #[derive(Debug)]
    struct VisitingPoint {
        x: i32,
        y: i32,
        transition: NavigationTransition,
    }

    let mut root_path = None;
    let mut visited_path_ids = HashSet::new();
    let mut visiting_paths = vec![Rc::new(RefCell::new(VisitingPath {
        inner: Some(path),
        inner_associated_point: None,
        inner_children_points: vec![],
        parent: None,
    }))];

    // Depth-first visiting
    while let Some(path) = visiting_paths.pop() {
        let mut path_mut = path.borrow_mut();

        // `path_mut` is not pre-processed yet. Pre-process by draining all of
        // `path_mut.inner.points`.
        if path_mut
            .inner
            .as_ref()
            .is_some_and(|inner| !inner.points.is_empty())
        {
            let inner = path_mut.inner.as_mut().expect("has value");
            // Non-root (leaf) path may not have next path
            if !visited_path_ids.insert(inner.id.ok_or(anyhow!("invalid path id"))?) {
                bail!("cycle detected when updating path");
            }

            // TODO: Check for other way to avoid borrow-checker
            let mut visiting_paths_extend = vec![];
            let points = inner.points.drain(..).collect::<Vec<_>>();

            for point in points {
                let next_path = point.next_path_id.and_then(|id| source.query_path(id).ok());
                let associated_point = VisitingPoint {
                    x: point.x,
                    y: point.y,
                    transition: point.transition,
                };

                visiting_paths_extend.push(Rc::new(RefCell::new(VisitingPath {
                    inner: next_path,
                    inner_associated_point: Some(associated_point),
                    inner_children_points: vec![],
                    parent: Some(path.clone()),
                })));
            }

            // Push this again for later processing
            // TODO: Check how to borrow mutably and pop later in the same iteration
            drop(path_mut);
            visiting_paths.push(path);
            visiting_paths.extend(visiting_paths_extend);
            continue;
        }

        // Non-root (leaf) path
        if let Some(point) = path_mut.inner_associated_point.take() {
            let mut point_inner = Point {
                next_path: None,
                x: point.x,
                y: point.y,
                transition: point.transition,
            };
            let parent = path_mut
                .parent
                .clone()
                .ok_or(anyhow!("non-root path without parent"))?;

            // The next path this `point` transitions to if any
            if let Some(path) = path_mut.inner.take() {
                point_inner.next_path = Some(Path {
                    id: path.id.expect("has valid id"),
                    minimap_snapshot_base64: path.minimap_snapshot_base64.clone(),
                    name_snapshot_base64: path.name_snapshot_base64.clone(),
                    points: path_mut.inner_children_points.drain(..).collect(),
                });
            }

            parent.borrow_mut().inner_children_points.push(point_inner);
            continue;
        }

        if root_path.is_none() {
            drop(path_mut); // For moving `path` into `root_path`
            root_path = Some(path);
        } else {
            bail!("duplicate root path");
        }
    }

    let mut root_path = Rc::into_inner(root_path.expect("valid root path"))
        .expect("no remaining borrow")
        .into_inner();
    let root_path_inner = root_path.inner.take().expect("valid root path's inner");

    Ok(Path {
        id: root_path_inner.id.expect("has valid id"),
        minimap_snapshot_base64: root_path_inner.minimap_snapshot_base64.clone(),
        name_snapshot_base64: root_path_inner.name_snapshot_base64.clone(),
        points: root_path.inner_children_points,
    })
}

fn find_current_from_base_path(
    base_path: &Path,
    detector: &dyn Detector,
    minimap_bbox: Rect,
    minimap_name_bbox: Rect,
) -> Result<Path> {
    let mut visiting_paths = vec![base_path];

    while let Some(path) = visiting_paths.pop() {
        let name_mat = decode_base64_to_mat(&path.name_snapshot_base64, true)?;
        let minimap_mat = decode_base64_to_mat(&path.minimap_snapshot_base64, false)?;

        if detector.detect_minimap_match(&minimap_mat, &name_mat, minimap_bbox, minimap_name_bbox) {
            return Ok(path.clone());
        }

        for point in &path.points {
            if let Some(path) = point.next_path.as_ref() {
                visiting_paths.push(path);
            }
        }
    }

    bail!("unable to determine current path")
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

fn find_root_paths(paths: Vec<NavigationPath>) -> Vec<NavigationPath> {
    let all_path_ids = paths
        .iter()
        .filter_map(|path| path.id)
        .collect::<HashSet<_>>();
    let referenced_path_ids = paths
        .iter()
        .flat_map(|point| &point.points)
        .filter_map(|point| point.next_path_id)
        .collect::<HashSet<_>>();
    let root_path_ids = all_path_ids
        .difference(&referenced_path_ids)
        .copied()
        .collect::<HashSet<_>>();

    paths
        .into_iter()
        .filter(|p| p.id.is_some_and(|id| root_path_ids.contains(&id)))
        .collect()
}

#[cfg(test)]
mod tests {
    use mockall::predicate::eq;

    use super::MockNavigatorDataSource;
    use super::*;
    use crate::database::NavigationPoint;

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

        // Path B → C
        let path_b_id = 2;
        let path_b = mock_navigation_path(
            Some(path_b_id),
            vec![NavigationPoint {
                next_path_id: Some(path_c_id),
                x: 20,
                y: 20,
                transition: NavigationTransition::Portal,
            }],
        );

        // Path A → B, D
        let path_a_id = 1;
        let path_a = mock_navigation_path(
            Some(path_a_id),
            vec![
                NavigationPoint {
                    next_path_id: Some(path_b_id),
                    x: 10,
                    y: 10,
                    transition: NavigationTransition::Portal,
                },
                NavigationPoint {
                    next_path_id: Some(path_d_id),
                    x: 11,
                    y: 10,
                    transition: NavigationTransition::Portal,
                },
            ],
        );

        let mut source = MockNavigatorDataSource::new();
        source
            .expect_query_path()
            .with(eq(path_b_id))
            .returning(move |_| Ok(path_b.clone()));
        source
            .expect_query_path()
            .with(eq(path_c_id))
            .returning(move |_| Ok(path_c.clone()));
        source
            .expect_query_path()
            .with(eq(path_d_id))
            .returning(move |_| Ok(path_d.clone()));
        source
            .expect_query_path()
            .with(eq(path_e_id))
            .returning(move |_| Ok(path_e.clone()));

        // Check structure
        let path = build_base_path_from(&source, path_a.clone()).expect("success");
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
        assert!(d_path.points.is_empty());

        let b_path = path.points[1]
            .next_path
            .as_ref()
            .expect("Path B should exist");

        let c_path = b_path.points[0]
            .next_path
            .as_ref()
            .expect("Path C should exist");
        assert_eq!(c_path.points.len(), 1);

        // Path E
        assert_eq!(c_path.points[0].x, 30);
        assert_eq!(c_path.points[0].y, 30);
        assert_eq!(c_path.points[0].transition, NavigationTransition::Portal);

        let e_path = c_path.points[0]
            .next_path
            .as_ref()
            .expect("Path E should exist");
        assert!(e_path.points.is_empty());
    }

    #[test]
    fn build_base_path_from_cycle_detection() {
        let path_b_id = 2;
        let path_b = mock_navigation_path(
            Some(path_b_id),
            vec![NavigationPoint {
                next_path_id: Some(1), // cycle back to A
                x: 10,
                y: 20,
                transition: NavigationTransition::Portal,
            }],
        );

        let path_a_id = 1;
        let path_a = mock_navigation_path(
            Some(path_a_id),
            vec![NavigationPoint {
                next_path_id: Some(2),
                x: 1,
                y: 2,
                transition: NavigationTransition::Portal,
            }],
        );

        let path_a_clone = path_a.clone();
        let mut source = MockNavigatorDataSource::new();
        source
            .expect_query_path()
            .with(eq(path_a_id))
            .returning(move |_| Ok(path_a.clone()));
        source
            .expect_query_path()
            .with(eq(path_b_id))
            .returning(move |_| Ok(path_b.clone()));

        let result = build_base_path_from(&source, path_a_clone);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("cycle detected"),
            "Expected cycle detection error, got: {err}"
        );
    }

    #[test]
    fn find_root_paths_single_root() {
        let path_c = mock_navigation_path(Some(3), vec![]);
        // Path B → C
        let path_b = mock_navigation_path(
            Some(2),
            vec![NavigationPoint {
                next_path_id: Some(3),
                x: 20,
                y: 20,
                transition: NavigationTransition::Portal,
            }],
        );
        // Path A → B
        let path_a = mock_navigation_path(
            Some(1),
            vec![NavigationPoint {
                next_path_id: Some(2),
                x: 10,
                y: 10,
                transition: NavigationTransition::Portal,
            }],
        );

        let paths = vec![path_a.clone(), path_b, path_c];
        let roots = find_root_paths(paths);

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, Some(1)); // Only path A is not referenced by others
    }

    #[test]
    fn find_root_paths_multiple_roots() {
        let path_a = mock_navigation_path(Some(1), vec![]); // No references
        let path_b = mock_navigation_path(Some(2), vec![]); // No references
        // Path C → A
        let path_c = mock_navigation_path(
            Some(3),
            vec![NavigationPoint {
                next_path_id: Some(1),
                x: 0,
                y: 0,
                transition: NavigationTransition::Portal,
            }],
        );

        let paths = vec![path_a.clone(), path_b.clone(), path_c];
        let roots = find_root_paths(paths);

        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].id, Some(2));
        assert_eq!(roots[1].id, Some(3));
    }

    #[test]
    fn find_root_paths_with_missing_ids() {
        let path_with_no_id = mock_navigation_path(None, vec![]);
        let path_with_id = mock_navigation_path(Some(1), vec![]);

        let paths = vec![path_with_no_id.clone(), path_with_id.clone()];
        let roots = find_root_paths(paths);

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, Some(1));
    }

    #[test]
    fn find_root_paths_empty_input() {
        let roots = find_root_paths(vec![]);
        assert!(roots.is_empty());
    }
}
