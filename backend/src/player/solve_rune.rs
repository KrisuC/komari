use platforms::windows::KeyKind;

use super::{
    Player, PlayerState,
    actions::PlayerAction,
    timeout::{Lifecycle, next_timeout_lifecycle},
};
use crate::{
    context::Context,
    detect::{ArrowsCalibrating, ArrowsState},
    player::{on_action_state_mut, timeout::Timeout},
};

const MAX_RETRY_COUNT: u32 = 2;

/// Representing the current stage of rune solving.
#[derive(Debug, Default, Clone, Copy)]
pub enum RuneStage {
    // Ensures stationary and all keys cleared before solving.
    #[default]
    Precondition,
    // Finds the region containing the four arrows.
    FindRegion(ArrowsCalibrating, Timeout, Option<Timeout>, u32),
    // Solves for the rune arrows that possibly include spinning arrows.
    Solving(ArrowsCalibrating, Timeout),
    // Presses the keys.
    PressKeys(Timeout, [KeyKind; 4], usize),
    // Terminal stage.
    Completed,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct SolvingRune {
    stage: RuneStage,
}

impl SolvingRune {
    #[inline]
    fn stage_precondition(self) -> SolvingRune {
        SolvingRune {
            stage: RuneStage::Precondition,
        }
    }

    #[inline]
    fn stage_find_region(
        self,
        calibrating: ArrowsCalibrating,
        timeout: Timeout,
        cooldown_timeout: Option<Timeout>,
        retry_count: u32,
    ) -> SolvingRune {
        SolvingRune {
            stage: RuneStage::FindRegion(calibrating, timeout, cooldown_timeout, retry_count),
        }
    }

    #[inline]
    fn stage_solving(self, calibrating: ArrowsCalibrating, timeout: Timeout) -> SolvingRune {
        SolvingRune {
            stage: RuneStage::Solving(calibrating, timeout),
        }
    }

    #[inline]
    fn stage_press_keys(
        self,
        timeout: Timeout,
        keys: [KeyKind; 4],
        key_index: usize,
    ) -> SolvingRune {
        SolvingRune {
            stage: RuneStage::PressKeys(timeout, keys, key_index),
        }
    }

    #[inline]
    fn stage_completed(self) -> SolvingRune {
        SolvingRune {
            stage: RuneStage::Completed,
        }
    }
}

/// Updates the [`Player::SolvingRune`] contextual state.
///
/// Note: This state does not use any [`Task`], so all detections are blocking. But this should be
/// acceptable for this state.
pub fn update_solving_rune_context(
    context: &Context,
    state: &mut PlayerState,
    solving_rune: SolvingRune,
) -> Player {
    let solving_rune = match solving_rune.stage {
        RuneStage::Precondition => {
            if !state.is_stationary || !context.keys.all_keys_cleared() {
                solving_rune.stage_precondition()
            } else {
                solving_rune.stage_find_region(
                    ArrowsCalibrating::default(),
                    Timeout::default(),
                    None,
                    0,
                )
            }
        }
        RuneStage::FindRegion(calibrating, timeout, cooldown_timeout, retry_count) => {
            update_find_region(
                context,
                solving_rune,
                state.config.interact_key,
                calibrating,
                timeout,
                cooldown_timeout,
                retry_count,
            )
        }
        RuneStage::Solving(calibrating, timeout) => {
            update_solving(context, solving_rune, calibrating, timeout)
        }
        RuneStage::PressKeys(timeout, keys, key_index) => {
            update_press_keys(context, solving_rune, timeout, keys, key_index)
        }
        RuneStage::Completed => unreachable!(),
    };
    let next = if matches!(solving_rune.stage, RuneStage::Completed) {
        Player::Idle
    } else {
        Player::SolvingRune(solving_rune)
    };

    on_action_state_mut(
        state,
        |state, action| match action {
            PlayerAction::SolveRune => {
                let is_terminal = matches!(next, Player::Idle);
                if is_terminal {
                    state.rune_validate_timeout = Some(Timeout::default());
                }
                Some((next, is_terminal))
            }
            PlayerAction::PingPong(_)
            | PlayerAction::AutoMob(_)
            | PlayerAction::Panic(_)
            | PlayerAction::Key(_)
            | PlayerAction::FamiliarsSwapping(_)
            | PlayerAction::Move(_) => {
                unreachable!()
            }
        },
        || Player::Idle, // Force cancel if not initiated from action
    )
}

fn update_find_region(
    context: &Context,
    solving_rune: SolvingRune,
    interact_key: KeyKind,
    calibrating: ArrowsCalibrating,
    timeout: Timeout,
    cooldown_timeout: Option<Timeout>,
    retry_count: u32,
) -> SolvingRune {
    // cooldown_timeout is used to wait for rune cooldown around ~4 secs before hitting interact
    // key again.
    if let Some(cooldown_timeout) = cooldown_timeout {
        return match next_timeout_lifecycle(cooldown_timeout, 125) {
            Lifecycle::Updated(cooldown_timeout) | Lifecycle::Started(cooldown_timeout) => {
                solving_rune.stage_find_region(
                    calibrating,
                    timeout,
                    Some(cooldown_timeout),
                    retry_count,
                )
            }
            Lifecycle::Ended => {
                solving_rune.stage_find_region(calibrating, timeout, None, retry_count)
            }
        };
    }

    debug_assert!(cooldown_timeout.is_none());
    match next_timeout_lifecycle(timeout, 35) {
        Lifecycle::Started(timeout) => {
            let _ = context.keys.send(interact_key);
            solving_rune.stage_find_region(calibrating, timeout, cooldown_timeout, retry_count)
        }
        Lifecycle::Ended => match context.detector_unwrap().detect_rune_arrows(calibrating) {
            Ok(ArrowsState::Calibrating(calibrating)) => {
                solving_rune.stage_solving(calibrating, Timeout::default())
            }
            Ok(ArrowsState::Complete(_)) => unreachable!(),
            Err(_) => {
                if retry_count < MAX_RETRY_COUNT {
                    // Retry possibly because mis-pressing the interact key
                    solving_rune.stage_find_region(
                        ArrowsCalibrating::default(),
                        Timeout::default(),
                        Some(Timeout::default()),
                        retry_count + 1,
                    )
                } else {
                    solving_rune.stage_completed()
                }
            }
        },
        Lifecycle::Updated(timeout) => {
            solving_rune.stage_find_region(calibrating, timeout, cooldown_timeout, retry_count)
        }
    }
}

fn update_solving(
    context: &Context,
    solving_rune: SolvingRune,
    calibrating: ArrowsCalibrating,
    timeout: Timeout,
) -> SolvingRune {
    match next_timeout_lifecycle(timeout, 150) {
        Lifecycle::Started(timeout) => solving_rune.stage_solving(calibrating, timeout),
        Lifecycle::Ended => solving_rune.stage_completed(),
        Lifecycle::Updated(timeout) => {
            match context.detector_unwrap().detect_rune_arrows(calibrating) {
                Ok(ArrowsState::Calibrating(calibrating)) => {
                    solving_rune.stage_solving(calibrating, timeout)
                }
                Ok(ArrowsState::Complete(keys)) => {
                    solving_rune.stage_press_keys(Timeout::default(), keys, 0)
                }
                Err(_) => solving_rune.stage_completed(),
            }
        }
    }
}

fn update_press_keys(
    context: &Context,
    solving_rune: SolvingRune,
    timeout: Timeout,
    keys: [KeyKind; 4],
    key_index: usize,
) -> SolvingRune {
    const PRESS_KEY_INTERVAL: u32 = 8;

    match next_timeout_lifecycle(timeout, PRESS_KEY_INTERVAL) {
        Lifecycle::Started(timeout) => {
            let _ = context.keys.send(keys[key_index]);
            solving_rune.stage_press_keys(timeout, keys, key_index)
        }
        Lifecycle::Ended => {
            if key_index + 1 < keys.len() {
                solving_rune.stage_press_keys(Timeout::default(), keys, key_index + 1)
            } else {
                solving_rune.stage_completed()
            }
        }
        Lifecycle::Updated(timeout) => solving_rune.stage_press_keys(timeout, keys, key_index),
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use anyhow::{Ok, anyhow};
    use mockall::predicate::eq;

    use super::*;
    use crate::{
        bridge::MockKeySender,
        context::Context,
        detect::{ArrowsCalibrating, ArrowsState, MockDetector},
    };

    #[test]
    fn update_solving_rune_context_precondition_to_find_region_when_stationary_and_keys_cleared() {
        let mut keys = MockKeySender::default();
        keys.expect_all_keys_cleared().once().returning(|| true);
        let context = Context::new(Some(keys), None);
        let solving_rune = SolvingRune::default().stage_precondition();
        let mut state = PlayerState::default();
        state.priority_action = Some(PlayerAction::SolveRune); // Avoid cancellation
        state.is_stationary = true;

        let result = update_solving_rune_context(&context, &mut state, solving_rune);

        assert_matches!(
            result,
            Player::SolvingRune(SolvingRune {
                stage: RuneStage::FindRegion(_, _, None, 0)
            })
        );
    }

    #[test]
    fn update_find_region_to_solving_on_calibrating() {
        let mut detector = MockDetector::default();
        detector
            .expect_detect_rune_arrows()
            .return_once(|_| Ok(ArrowsState::Calibrating(ArrowsCalibrating::default())));
        let context = Context::new(None, Some(detector));
        let solving_rune = SolvingRune::default().stage_find_region(
            ArrowsCalibrating::default(),
            Timeout::default(),
            None,
            0,
        );

        let result = update_find_region(
            &context,
            solving_rune,
            KeyKind::default(),
            ArrowsCalibrating::default(),
            Timeout {
                started: true,
                current: 35,
                ..Default::default()
            },
            None,
            0,
        );

        assert_matches!(
            result,
            SolvingRune {
                stage: RuneStage::Solving(
                    _,
                    Timeout {
                        started: false,
                        current: 0,
                        ..
                    },
                )
            }
        );
    }

    #[test]
    fn update_find_region_retry() {
        let mut detector = MockDetector::default();
        detector
            .expect_detect_rune_arrows()
            .return_once(move |_| Err(anyhow!("rune region not found")));
        let context = Context::new(None, Some(detector));
        let solving_rune = SolvingRune::default().stage_find_region(
            ArrowsCalibrating::default(),
            Timeout::default(),
            None,
            0,
        );

        let result = update_find_region(
            &context,
            solving_rune,
            KeyKind::default(),
            ArrowsCalibrating::default(),
            Timeout {
                started: true,
                current: 35,
                ..Default::default()
            },
            None,
            0,
        );

        assert_matches!(
            result,
            SolvingRune {
                stage: RuneStage::FindRegion(
                    _,
                    Timeout { started: false, .. },
                    Some(Timeout { started: false, .. }),
                    1
                )
            }
        );
    }

    #[test]
    fn update_find_region_retry_cooldown_timeout_to_none() {
        let context = Context::new(None, None);
        let solving_rune = SolvingRune::default().stage_find_region(
            ArrowsCalibrating::default(),
            Timeout::default(),
            None,
            0,
        );

        let result = update_find_region(
            &context,
            solving_rune,
            KeyKind::default(),
            ArrowsCalibrating::default(),
            Timeout::default(),
            Some(Timeout {
                started: true,
                current: 125,
                ..Default::default()
            }),
            1,
        );

        assert_matches!(
            result,
            SolvingRune {
                stage: RuneStage::FindRegion(_, _, None, 1)
            }
        );
    }

    #[test]
    fn update_solving_to_completed_on_error() {
        let mut detector = MockDetector::default();
        detector
            .expect_detect_rune_arrows()
            .returning(|_| Err(anyhow::anyhow!("fail")));
        let context = Context::new(None, Some(detector));
        let solving_rune =
            SolvingRune::default().stage_solving(ArrowsCalibrating::default(), Timeout::default());

        let result = update_solving(
            &context,
            solving_rune,
            ArrowsCalibrating::default(),
            Timeout {
                started: true,
                ..Default::default()
            },
        );

        assert_matches!(
            result,
            SolvingRune {
                stage: RuneStage::Completed
            }
        );
    }

    #[test]
    fn update_solving_to_solving_on_incomplete() {
        let mut detector = MockDetector::default();
        detector
            .expect_detect_rune_arrows()
            .return_once(move |_| Ok(ArrowsState::Calibrating(ArrowsCalibrating::default())));
        let context = Context::new(None, Some(detector));
        let solving_rune =
            SolvingRune::default().stage_solving(ArrowsCalibrating::default(), Timeout::default());

        let result = update_solving(
            &context,
            solving_rune,
            ArrowsCalibrating::default(),
            Timeout {
                started: true,
                ..Default::default()
            },
        );

        assert_matches!(
            result,
            SolvingRune {
                stage: RuneStage::Solving(_, Timeout { started: true, .. })
            }
        );
    }

    #[test]
    fn update_solving_to_press_keys_on_complete() {
        let expected_keys = [KeyKind::A, KeyKind::S, KeyKind::D, KeyKind::F];
        let mut detector = MockDetector::default();
        detector
            .expect_detect_rune_arrows()
            .return_once(move |_| Ok(ArrowsState::Complete(expected_keys)));
        let context = Context::new(None, Some(detector));
        let solving_rune =
            SolvingRune::default().stage_solving(ArrowsCalibrating::default(), Timeout::default());

        let result = update_solving(
            &context,
            solving_rune,
            ArrowsCalibrating::default(),
            Timeout {
                started: true,
                ..Default::default()
            },
        );

        assert_matches!(
            result,
            SolvingRune {
                stage: RuneStage::PressKeys(
                    Timeout {
                        started: false,
                        current: 0,
                        ..
                    },
                    [KeyKind::A, KeyKind::S, KeyKind::D, KeyKind::F],
                    0
                )
            }
        );
    }

    #[test]
    fn update_press_keys_to_completed_after_all_keys_sent() {
        let expected_keys = [KeyKind::A, KeyKind::S, KeyKind::D, KeyKind::F];
        let mut key_index = 0;

        // Simulate 4 rounds of key pressing
        for _ in 0..4 {
            let mut keys = MockKeySender::default();
            keys.expect_send()
                .with(eq(expected_keys[key_index]))
                .return_once(|_| Ok(()));
            let context = Context::new(Some(keys), None);

            // Press the key
            update_press_keys(
                &context,
                SolvingRune::default(),
                Timeout::default(),
                expected_keys,
                key_index,
            );
            // Timing out and advance key index or complete
            let end_result = update_press_keys(
                &context,
                SolvingRune::default(),
                Timeout {
                    current: 8,
                    started: true,
                    ..Default::default()
                },
                expected_keys,
                key_index,
            );

            if key_index == expected_keys.len() - 1 {
                assert_matches!(
                    end_result,
                    SolvingRune {
                        stage: RuneStage::Completed
                    }
                );
            } else {
                key_index = match end_result.stage {
                    RuneStage::PressKeys(_, _, index) => index,
                    _ => unreachable!(),
                }
            }
        }
    }
}
