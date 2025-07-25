use crate::{Character, PotionMode, database::Minimap as MinimapData, player::PlayerState};

#[derive(Debug, Default)]
pub struct PlayerService {
    character: Option<Character>,
}

/// A service to handle player-related incoming requests.
impl PlayerService {
    pub fn update(&mut self, character: Option<Character>) {
        self.character = character;
    }

    pub fn current(&self) -> Option<&Character> {
        self.character.as_ref()
    }

    /// Updates `state` from currently using `minimap`.
    pub fn update_from_minimap(&self, state: &mut PlayerState, minimap: Option<&MinimapData>) {
        state.reset();
        if let Some(minimap) = minimap {
            state.config.rune_platforms_pathing = minimap.rune_platforms_pathing;
            state.config.rune_platforms_pathing_up_jump_only =
                minimap.rune_platforms_pathing_up_jump_only;
            state.config.auto_mob_platforms_pathing = minimap.auto_mob_platforms_pathing;
            state.config.auto_mob_platforms_pathing_up_jump_only =
                minimap.auto_mob_platforms_pathing_up_jump_only;
            state.config.auto_mob_platforms_bound = minimap.auto_mob_platforms_bound;
        }
    }

    /// Updates `state` from currently using `[Character]`.
    pub fn update_from_character(&self, state: &mut PlayerState) {
        state.reset();
        if let Some(character) = self.character.as_ref() {
            state.config.class = character.class;
            state.config.disable_adjusting = character.disable_adjusting;
            state.config.interact_key = character.interact_key.key.into();
            state.config.grappling_key = character.ropelift_key.map(|key| key.key.into());
            state.config.teleport_key = character.teleport_key.map(|key| key.key.into());
            state.config.jump_key = character.jump_key.key.into();
            state.config.upjump_key = character.up_jump_key.map(|key| key.key.into());
            state.config.cash_shop_key = character.cash_shop_key.key.into();
            state.config.familiar_key = character.familiar_menu_key.key.into();
            state.config.to_town_key = character.to_town_key.key.into();
            state.config.change_channel_key = character.change_channel_key.key.into();
            state.config.potion_key = character.potion_key.key.into();
            state.config.use_potion_below_percent =
                match (character.potion_key.enabled, character.potion_mode) {
                    (false, _) | (_, PotionMode::EveryMillis(_)) => None,
                    (_, PotionMode::Percentage(percent)) => Some(percent / 100.0),
                };
            state.config.update_health_millis = Some(character.health_update_millis);
        }
    }
}
