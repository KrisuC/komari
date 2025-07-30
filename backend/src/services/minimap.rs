#[cfg(test)]
use mockall::automock;

use crate::{
    context::Context,
    database::Minimap as MinimapData,
    minimap::{Minimap, MinimapState},
    pathing::Platform,
};

#[derive(Debug, Default)]
pub struct MinimapService {
    minimap: Option<MinimapData>,
    preset: Option<String>,
}

/// A service to handle minimap-related incoming requests.
#[cfg_attr(test, automock)]
impl MinimapService {
    /// Creates a new [`MinimapData`] from currently detected minimap with `name`.
    pub fn create(&self, context: &Context, name: String) -> Option<MinimapData> {
        if let Minimap::Idle(idle) = context.minimap {
            Some(MinimapData {
                name,
                width: idle.bbox.width,
                height: idle.bbox.height,
                ..MinimapData::default()
            })
        } else {
            None
        }
    }

    #[allow(clippy::needless_lifetimes)]
    pub fn current<'a>(&'a self) -> Option<&'a MinimapData> {
        self.minimap.as_ref()
    }

    pub fn current_preset(&self) -> Option<String> {
        self.preset.clone()
    }

    /// Updates the currently used `state`, `minimap` and `preset` from
    /// `new_preset` and `new_minimap`.
    pub fn update(
        &mut self,
        state: &mut MinimapState,
        preset: Option<String>,
        minimap: Option<MinimapData>,
    ) {
        self.minimap = minimap;
        self.preset = preset;

        let platforms = self
            .minimap
            .as_ref()
            .map(|data| {
                data.platforms
                    .iter()
                    .copied()
                    .map(Platform::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        state.set_platforms(platforms);
    }

    /// Re-detects current minimap.
    pub fn redetect(&self, context: &mut Context) {
        context.minimap = Minimap::Detecting;
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use opencv::core::Rect;

    use super::*;
    use crate::{
        Platform as DatabasePlatform,
        context::Context,
        minimap::{Minimap, MinimapIdle, MinimapState},
        pathing::Platform,
    };

    fn mock_idle_minimap() -> MinimapIdle {
        let mut idle = MinimapIdle::default();
        idle.bbox = Rect::new(0, 0, 100, 100);
        idle
    }

    fn mock_minimap_data() -> MinimapData {
        MinimapData {
            name: "MapData".to_string(),
            width: 100,
            height: 100,
            ..Default::default()
        }
    }

    #[test]
    fn create_returns_some_when_idle_minimap() {
        let service = MinimapService::default();
        let mut context = Context::new(None, None);
        context.minimap = Minimap::Idle(mock_idle_minimap());

        let result = service.create(&context, "MapData".to_string());

        assert!(result.is_some());
        assert_eq!(result.unwrap(), mock_minimap_data());
    }

    #[test]
    fn create_returns_none_when_not_idle_minimap() {
        let service = MinimapService::default();
        let context = Context::new(None, None);

        let result = service.create(&context, "ShouldNotExist".to_string());

        assert!(result.is_none());
    }

    #[test]
    fn update_with_minimap_and_preset() {
        let mut service = MinimapService::default();
        let mut state = MinimapState::default();
        let minimap = mock_minimap_data();
        let preset = Some("custom".to_string());

        service.update(&mut state, preset.clone(), Some(minimap.clone()));

        assert_eq!(service.minimap, Some(minimap));
        assert_eq!(service.preset, preset);
    }

    #[test]
    fn test_update_with_none_minimap_and_preset() {
        let mut service = MinimapService::default();
        let mut state = MinimapState::default();
        state.set_platforms(vec![Platform::from(DatabasePlatform {
            x_start: 3,
            x_end: 3,
            y: 10,
        })]);

        service.update(&mut state, None, None);

        assert!(service.minimap.is_none());
        assert!(service.preset.is_none());
        assert!(state.platforms().is_empty());
    }

    #[test]
    fn redetect_sets_minimap_to_detecting() {
        let service = MinimapService::default();
        let mut context = Context::new(None, None);
        context.minimap = Minimap::Idle(mock_idle_minimap());

        service.redetect(&mut context);

        assert_matches!(context.minimap, Minimap::Detecting);
    }
}
