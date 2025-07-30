#[cfg(test)]
use mockall::automock;
use mockall_double::double;

#[double]
use crate::rotator::Rotator;
use crate::{
    Action, Character, KeyBinding, Minimap, RotationMode, RotatorMode, Settings, buff::BuffKind,
    rotator::RotatorBuildArgs,
};

// TODO: Whether to use Rc<RefCell<Rotator>> like Settings
#[derive(Debug, Default)]
pub struct RotatorService;

/// A service to handle [`Rotator`]-related incoming requests.
#[cfg_attr(test, automock)]
impl RotatorService {
    /// Updates `rotator` with data from `minimap`, `character`, `settings`, `actions` and `buffs`.
    pub fn update<'a>(
        &self,
        rotator: &mut Rotator,
        minimap: Option<&'a Minimap>,
        character: Option<&'a Character>,
        settings: &Settings,
        actions: &[Action],
        buffs: &[(BuffKind, KeyBinding)],
    ) {
        let mode = rotator_mode_from(minimap);
        let reset_normal_actions_on_erda = minimap
            .map(|minimap| minimap.actions_any_reset_on_erda_condition)
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
            actions,
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use strum::IntoEnumIterator;

    use super::*;
    use crate::{
        Bound, EliteBossBehavior, FamiliarRarity, KeyBindingConfiguration, SwappableFamiliars,
    };

    #[test]
    fn update_rotator_mode() {
        let mut minimap = Minimap {
            rotation_auto_mob_bound: Bound {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            },
            rotation_ping_pong_bound: Bound {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            },
            ..Default::default()
        };
        let character = Character::default();
        let service = RotatorService;

        for mode in RotationMode::iter() {
            minimap.rotation_mode = mode;
            let mut rotator = Rotator::new();
            rotator
                .expect_build_actions()
                .withf(move |args| {
                    let mut key_bound = None;
                    let original_mode = match args.mode {
                        RotatorMode::StartToEnd => RotationMode::StartToEnd,
                        RotatorMode::StartToEndThenReverse => RotationMode::StartToEndThenReverse,
                        RotatorMode::AutoMobbing(key, bound) => {
                            key_bound = Some((key, bound));
                            RotationMode::AutoMobbing
                        }
                        RotatorMode::PingPong(key, bound) => {
                            key_bound = Some((key, bound));
                            RotationMode::PingPong
                        }
                    };
                    let key_bound_match = match key_bound {
                        Some((key, bound)) => {
                            let bound_match = if original_mode == RotationMode::AutoMobbing {
                                bound == minimap.rotation_auto_mob_bound
                            } else {
                                bound == minimap.rotation_ping_pong_bound
                            };
                            key == minimap.rotation_mobbing_key && bound_match
                        }
                        None => true,
                    };

                    mode == original_mode && key_bound_match
                })
                .once()
                .return_const(());

            service.update(
                &mut rotator,
                Some(&minimap),
                Some(&character),
                &Settings::default(),
                &[],
                &[],
            );
        }
    }

    #[test]
    fn update_with_buffs() {
        let buffs = vec![(BuffKind::SayramElixir, KeyBinding::F1)];

        let buffs_clone = buffs.clone();
        let mut rotator = Rotator::new();
        rotator
            .expect_build_actions()
            .withf(move |args| args.buffs == &buffs_clone)
            .once()
            .return_const(());

        let service = RotatorService;
        service.update(&mut rotator, None, None, &Settings::default(), &[], &buffs);
    }

    #[test]
    fn update_with_familiar_essence_key() {
        let character = Character {
            familiar_essence_key: KeyBindingConfiguration {
                key: KeyBinding::Z,
                enabled: true,
            },
            ..Default::default()
        };

        let mut rotator = Rotator::new();
        rotator
            .expect_build_actions()
            .withf(|args| args.familiar_essence_key == KeyBinding::Z)
            .once()
            .return_const(());

        let service = RotatorService;
        service.update(
            &mut rotator,
            None,
            Some(&character),
            &Settings::default(),
            &[],
            &[],
        );
    }

    #[test]
    fn update_with_familiar_swap_config() {
        let mut settings = Settings::default();
        settings.familiars.swappable_familiars = SwappableFamiliars::SecondAndLast;
        settings.familiars.swappable_rarities =
            HashSet::from_iter([FamiliarRarity::Epic, FamiliarRarity::Rare]);
        settings.familiars.swap_check_millis = 5000;
        settings.familiars.enable_familiars_swapping = true;

        let settings_clone = settings.clone();
        let mut rotator = Rotator::new();
        rotator
            .expect_build_actions()
            .withf(move |args| {
                args.familiar_swappable_slots == SwappableFamiliars::SecondAndLast
                    && args.familiar_swappable_rarities == &settings.familiars.swappable_rarities
                    && args.familiar_swap_check_millis == 5000
                    && args.enable_familiars_swapping
            })
            .once()
            .return_const(());

        let service = RotatorService;
        service.update(&mut rotator, None, None, &settings_clone, &[], &[]);
    }

    #[test]
    fn update_with_elite_boss_behavior() {
        let character = Character {
            elite_boss_behavior_enabled: true,
            elite_boss_behavior: EliteBossBehavior::CycleChannel,
            elite_boss_behavior_key: KeyBinding::X,
            ..Default::default()
        };

        let mut rotator = Rotator::new();
        rotator
            .expect_build_actions()
            .withf(|args| {
                args.elite_boss_behavior == Some(EliteBossBehavior::CycleChannel)
                    && args.elite_boss_behavior_key == KeyBinding::X
            })
            .once()
            .return_const(());

        let service = RotatorService;
        service.update(
            &mut rotator,
            None,
            Some(&character),
            &Settings::default(),
            &[],
            &[],
        );
    }

    #[test]
    fn update_with_reset_normal_actions_on_erda() {
        let minimap = Minimap {
            actions_any_reset_on_erda_condition: true,
            ..Default::default()
        };

        let mut rotator = Rotator::new();
        rotator
            .expect_build_actions()
            .withf(|args| args.enable_reset_normal_actions_on_erda)
            .once()
            .return_const(());

        let service = RotatorService;
        service.update(
            &mut rotator,
            Some(&minimap),
            None,
            &Settings::default(),
            &[],
            &[],
        );
    }

    #[test]
    fn update_with_panic_mode_and_rune_solving() {
        let mut settings = Settings::default();
        settings.enable_panic_mode = true;
        settings.enable_rune_solving = true;

        let mut rotator = Rotator::new();
        rotator
            .expect_build_actions()
            .withf(|args| args.enable_panic_mode && args.enable_rune_solving)
            .once()
            .return_const(());

        let service = RotatorService;
        service.update(&mut rotator, None, None, &settings, &[], &[]);
    }
}
