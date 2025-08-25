use std::fmt::Debug;

use base64::{Engine, prelude::BASE64_STANDARD};
use opencv::{
    core::{MatTraitConst, Rect, Vector},
    imgcodecs::{IMREAD_GRAYSCALE, imdecode, imencode_def},
};

use crate::{NavigationPath, context::Context, minimap::Minimap};

/// A service to handle navigation-related requests.
pub trait NavigatorService: Debug {
    /// Creates a new [`NavigationPath`] if minimap is currently [`Minimap::Idle`].
    fn create_path(&self, context: &Context) -> Option<NavigationPath>;

    /// Recaptures `path` with new information if minimap is currently [`Minimap::Idle`].
    ///
    /// Returns the updated [`NavigationPath`] or the original.
    fn recapture_path(&self, context: &Context, path: NavigationPath) -> NavigationPath;

    /// Converts image `base64` to grayscale.
    fn navigation_snapshot_as_grayscale(&self, base64: String) -> String;
}

/// Default implementation of [`NavigatorService`].
#[derive(Debug, Default)]
pub struct DefaultNavigatorService;

impl NavigatorService for DefaultNavigatorService {
    fn create_path(&self, context: &Context) -> Option<NavigationPath> {
        if let Some((minimap_base64, name_base64, name_bbox)) =
            extract_minimap_and_name_base64(context)
        {
            Some(NavigationPath {
                minimap_snapshot_base64: minimap_base64,
                name_snapshot_base64: name_base64,
                name_snapshot_width: name_bbox.width,
                name_snapshot_height: name_bbox.height,
                ..NavigationPath::default()
            })
        } else {
            None
        }
    }

    fn recapture_path(&self, context: &Context, mut path: NavigationPath) -> NavigationPath {
        if let Some((minimap_base64, name_base64, name_bbox)) =
            extract_minimap_and_name_base64(context)
        {
            path.minimap_snapshot_base64 = minimap_base64;
            path.name_snapshot_base64 = name_base64;
            path.name_snapshot_width = name_bbox.width;
            path.name_snapshot_height = name_bbox.height;
        }

        path
    }

    fn navigation_snapshot_as_grayscale(&self, base64: String) -> String {
        convert_color_base64_to_grayscale_base64(base64.clone()).unwrap_or(base64)
    }
}

fn convert_color_base64_to_grayscale_base64(base64: String) -> Option<String> {
    let bytes = BASE64_STANDARD.decode(base64).ok()?;
    let mut bytes = Vector::<u8>::from_iter(bytes);
    let mat = imdecode(&bytes, IMREAD_GRAYSCALE).ok()?;

    bytes.clear();
    imencode_def(".png", &mat, &mut bytes).ok()?;

    Some(BASE64_STANDARD.encode(bytes))
}

// TODO: Better way?
fn extract_minimap_and_name_base64(context: &Context) -> Option<(String, String, Rect)> {
    if let Minimap::Idle(idle) = context.minimap
        && let Some(detector) = context.detector.as_ref()
    {
        let name_bbox = detector.detect_minimap_name(idle.bbox).ok()?;
        let name = detector.grayscale_mat().roi(name_bbox).ok()?;
        let mut name_bytes = Vector::new();
        imencode_def(".png", &name, &mut name_bytes).ok()?;
        let name_base64 = BASE64_STANDARD.encode(name_bytes);

        let minimap = detector.mat().roi(idle.bbox).ok()?;
        let mut minimap_bytes = Vector::new();
        imencode_def(".png", &minimap, &mut minimap_bytes).ok()?;
        let minimap_base64 = BASE64_STANDARD.encode(minimap_bytes);

        Some((minimap_base64, name_base64, name_bbox))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {}
