#[cfg(test)]
use mockall::{automock, concretize};

use crate::{Character, PotionMode, database::Minimap as MinimapData, player::PlayerState};

#[derive(Debug, Default)]
pub struct PlayerService {
    character: Option<Character>,
}

/// A service to handle player-related incoming requests.
#[cfg_attr(test, automock)]
impl PlayerService {
    pub fn update(&mut self, character: Option<Character>) {
        self.character = character;
    }

    #[allow(clippy::needless_lifetimes)]
    pub fn current<'a>(&'a self) -> Option<&'a Character> {
        self.character.as_ref()
    }

    /// Updates `state` from currently using `minimap`.
    #[cfg_attr(test, concretize)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Class, KeyBinding, KeyBindingConfiguration, bridge::KeyKind, database::Minimap,
        player::PlayerState,
    };

    fn mock_character() -> Character {
        Character {
            class: Class::Cadena,
            disable_adjusting: true,
            interact_key: KeyBindingConfiguration {
                key: KeyBinding::Z,
                ..Default::default()
            },
            ropelift_key: Some(KeyBindingConfiguration {
                key: KeyBinding::V,
                ..Default::default()
            }),
            teleport_key: Some(KeyBindingConfiguration {
                key: KeyBinding::X,
                ..Default::default()
            }),
            jump_key: KeyBindingConfiguration {
                key: KeyBinding::C,
                ..Default::default()
            },
            up_jump_key: Some(KeyBindingConfiguration {
                key: KeyBinding::A,
                ..Default::default()
            }),
            cash_shop_key: KeyBindingConfiguration {
                key: KeyBinding::B,
                ..Default::default()
            },
            familiar_menu_key: KeyBindingConfiguration {
                key: KeyBinding::N,
                ..Default::default()
            },
            to_town_key: KeyBindingConfiguration {
                key: KeyBinding::M,
                ..Default::default()
            },
            change_channel_key: KeyBindingConfiguration {
                key: KeyBinding::L,
                ..Default::default()
            },
            potion_key: KeyBindingConfiguration {
                key: KeyBinding::P,
                enabled: true,
            },
            potion_mode: PotionMode::Percentage(50.0),
            health_update_millis: 3000,
            ..Default::default()
        }
    }

    fn mock_minimap() -> Minimap {
        Minimap {
            rune_platforms_pathing: true,
            rune_platforms_pathing_up_jump_only: true,
            auto_mob_platforms_pathing: true,
            auto_mob_platforms_bound: true,
            ..Default::default()
        }
    }

    #[test]
    fn update_and_current() {
        let mut service = PlayerService::default();
        assert!(service.current().is_none());

        let character = mock_character();
        service.update(Some(character.clone()));
        let current = service.current().unwrap();

        assert_eq!(current, &mock_character());
    }

    #[test]
    fn update_from_minimap_none() {
        let service = PlayerService::default();
        let mut state = PlayerState::default();
        state.config.rune_platforms_pathing = true;
        state.config.rune_platforms_pathing_up_jump_only = true;

        service.update_from_minimap(&mut state, None);
        assert!(state.config.rune_platforms_pathing); // Doesn't change
        assert!(state.config.rune_platforms_pathing_up_jump_only); // Doesn't change
    }

    #[test]
    fn update_from_minimap_some() {
        let service = PlayerService::default();
        let minimap = mock_minimap();
        let mut state = PlayerState::default();

        service.update_from_minimap(&mut state, Some(&minimap));

        assert!(state.config.rune_platforms_pathing);
        assert!(state.config.rune_platforms_pathing_up_jump_only);
        assert!(state.config.auto_mob_platforms_pathing);
        assert!(state.config.auto_mob_platforms_bound);
    }

    #[test]
    fn update_from_character_none() {
        let service = PlayerService::default();
        let mut state = PlayerState::default();
        state.config.class = Class::Blaster;

        service.update_from_character(&mut state);
        assert_eq!(state.config.class, Class::Blaster);
    }

    #[test]
    fn update_from_character_some() {
        let mut service = PlayerService::default();
        let character = mock_character();
        service.update(Some(character.clone()));

        let mut state = PlayerState::default();
        service.update_from_character(&mut state);

        assert_eq!(state.config.class, character.class);
        assert_eq!(state.config.disable_adjusting, character.disable_adjusting);
        assert_eq!(state.config.interact_key, KeyKind::Z);
        assert_eq!(state.config.grappling_key, Some(KeyKind::V));
        assert_eq!(state.config.teleport_key, Some(KeyKind::X));
        assert_eq!(state.config.jump_key, KeyKind::C);
        assert_eq!(state.config.upjump_key, Some(KeyKind::A));
        assert_eq!(state.config.cash_shop_key, KeyKind::B);
        assert_eq!(state.config.familiar_key, KeyKind::N);
        assert_eq!(state.config.to_town_key, KeyKind::M);
        assert_eq!(state.config.change_channel_key, KeyKind::L);
        assert_eq!(state.config.potion_key, KeyKind::P);
        assert_eq!(state.config.use_potion_below_percent, Some(0.5));
        assert_eq!(state.config.update_health_millis, Some(3000));
    }
}
