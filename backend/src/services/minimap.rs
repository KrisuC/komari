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

    pub fn current(&self) -> Option<&MinimapData> {
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
mod tests {}
