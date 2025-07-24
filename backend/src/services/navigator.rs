use base64::{Engine, prelude::BASE64_STANDARD};
use opencv::{
    core::{MatTraitConst, Rect, Vector},
    imgcodecs::imencode_def,
};

use crate::{NavigationPath, context::Context, minimap::Minimap};

#[derive(Debug)]
pub struct NavigatorService;

impl NavigatorService {
    pub fn create_path(&self, context: &Context) -> Option<NavigationPath> {
        if let Some((minimap_base64, name_base64, name_bbox)) =
            extract_minimap_and_name_base64(context)
        {
            Some(NavigationPath {
                minimap_snapshot_base64: minimap_base64,
                name_snapshot_base64: name_base64,
                name_snapshot_width: name_bbox.width,
                name_snapshot_height: name_bbox.height,
                points: vec![],
            })
        } else {
            None
        }
    }

    pub fn recapture_path(&self, context: &Context, mut path: NavigationPath) -> NavigationPath {
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
