use crate::{
    Action, ActionCondition, ActionConfigurationCondition, ActionKey, Character, KeyBinding,
    KeyBindingConfiguration, Minimap, PotionMode, RotationMode, RotatorMode, Settings,
    buff::BuffKind,
    rotator::{Rotator, RotatorBuildArgs},
};

// TODO: Whether to use Rc<RefCell<Rotator>> like Settings
#[derive(Debug)]
pub struct RotatorService;

/// A service to handle [`Rotator`]-related incoming requests.
impl RotatorService {
    /// Updates `rotator` with data from `minimap`, `character`, `settings`, `actions` and `buffs`.
    pub fn update(
        &self,
        rotator: &mut Rotator,
        minimap: Option<&Minimap>,
        character: Option<&Character>,
        settings: &Settings,
        actions: &[Action],
        buffs: &[(BuffKind, KeyBinding)],
    ) {
        let mode = rotator_mode_from(minimap);
        let reset_normal_actions_on_erda = minimap
            .map(|minimap| minimap.actions_any_reset_on_erda_condition)
            .unwrap_or_default();
        let actions = character
            .map(|character| {
                actions_from(character)
                    .into_iter()
                    .chain(actions.iter().copied())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let familiar_essence_key = character
            .map(|character| character.familiar_essence_key.key)
            .unwrap_or_default();
        let elite_boss_behavior = character.and_then(|character| {
            character
                .elite_boss_behavior_enabled
                .then_some(character.elite_boss_behavior)
        });
        let elite_boss_behavior_key = character
            .map(|character| character.elite_boss_behavior_key)
            .unwrap_or_default();
        let args = RotatorBuildArgs {
            mode,
            actions: actions.as_slice(),
            buffs,
            familiar_essence_key,
            familiar_swappable_slots: settings.familiars.swappable_familiars,
            familiar_swappable_rarities: &settings.familiars.swappable_rarities,
            familiar_swap_check_millis: settings.familiars.swap_check_millis,
            elite_boss_behavior,
            elite_boss_behavior_key,
            enable_panic_mode: settings.enable_panic_mode,
            enable_rune_solving: settings.enable_rune_solving,
            enable_familiars_swapping: settings.familiars.enable_familiars_swapping,
            enable_reset_normal_actions_on_erda: reset_normal_actions_on_erda,
        };

        rotator.build_actions(args);
    }
}

#[inline]
fn rotator_mode_from(minimap: Option<&Minimap>) -> RotatorMode {
    minimap
        .map(|minimap| match minimap.rotation_mode {
            RotationMode::StartToEnd => RotatorMode::StartToEnd,
            RotationMode::StartToEndThenReverse => RotatorMode::StartToEndThenReverse,
            RotationMode::AutoMobbing => RotatorMode::AutoMobbing(
                minimap.rotation_mobbing_key,
                minimap.rotation_auto_mob_bound,
            ),
            RotationMode::PingPong => RotatorMode::PingPong(
                minimap.rotation_mobbing_key,
                minimap.rotation_ping_pong_bound,
            ),
        })
        .unwrap_or_default()
}

fn actions_from(character: &Character) -> Vec<Action> {
    let mut vec = Vec::new();

    if let KeyBindingConfiguration { key, enabled: true } = character.feed_pet_key {
        let feed_pet_action = Action::Key(ActionKey {
            key,
            count: 1,
            condition: ActionCondition::EveryMillis(character.feed_pet_millis),
            wait_before_use_millis: 350,
            wait_after_use_millis: 350,
            ..ActionKey::default()
        });
        for _ in 0..character.num_pets {
            vec.push(feed_pet_action);
        }
    }

    if let KeyBindingConfiguration { key, enabled: true } = character.potion_key
        && let PotionMode::EveryMillis(millis) = character.potion_mode
    {
        vec.push(Action::Key(ActionKey {
            key,
            count: 1,
            condition: ActionCondition::EveryMillis(millis),
            wait_before_use_millis: 350,
            wait_after_use_millis: 350,
            ..ActionKey::default()
        }));
    }

    let mut i = 0;
    let config_actions = &character.actions;
    while i < config_actions.len() {
        let action = config_actions[i];
        let enabled = action.enabled;

        if enabled {
            vec.push(action.into());
        }
        while i + 1 < config_actions.len() {
            let action = config_actions[i + 1];
            if !matches!(action.condition, ActionConfigurationCondition::Linked) {
                break;
            }
            if enabled {
                vec.push(action.into());
            }
            i += 1;
        }

        i += 1;
    }
    vec
}

#[cfg(test)]
mod tests {}
