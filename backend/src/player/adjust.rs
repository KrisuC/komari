use std::cmp::Ordering;

use platforms::windows::KeyKind;

use super::{
    PlayerAction, PlayerActionKey, PlayerState,
    moving::Moving,
    timeout::{Lifecycle, next_timeout_lifecycle},
    use_key::UseKey,
};
use crate::{
    ActionKeyDirection, ActionKeyWith,
    context::Context,
    player::{
        Player,
        actions::{on_action_state, on_auto_mob_use_key_action},
        double_jump::DoubleJumping,
        moving::MOVE_TIMEOUT,
        state::LastMovement,
        timeout::{ChangeAxis, MovingLifecycle, Timeout, next_moving_lifecycle_with_axis},
    },
};

/// Minimum x distance from the destination required to perform small movement.
pub const ADJUSTING_SHORT_THRESHOLD: i32 = 1;

/// Minimum x distance from the destination required to walk.
pub const ADJUSTING_MEDIUM_THRESHOLD: i32 = 3;

const ADJUSTING_SHORT_TIMEOUT: u32 = MOVE_TIMEOUT + 1;

/// Minimium y distance required to perform a fall and then walk.
const FALLING_THRESHOLD: i32 = 8;

#[derive(Clone, Copy, Debug)]
pub struct Adjusting {
    pub moving: Moving,
    adjust_timeout: Timeout,
}

impl Adjusting {
    pub fn new(moving: Moving) -> Self {
        Self {
            moving,
            adjust_timeout: Timeout::default(),
        }
    }

    fn moving(self, moving: Moving) -> Adjusting {
        Adjusting { moving, ..self }
    }

    fn update_adjusting(&mut self, context: &Context, up_key: KeyKind, down_key: KeyKind) {
        self.adjust_timeout =
            match next_timeout_lifecycle(self.adjust_timeout, ADJUSTING_SHORT_TIMEOUT) {
                Lifecycle::Started(timeout) => {
                    let _ = context.keys.send_up(up_key);
                    let _ = context.keys.send(down_key);
                    timeout
                }
                Lifecycle::Ended => Timeout::default(),
                Lifecycle::Updated(timeout) => timeout,
            };
    }
}

/// Updates the [`Player::Adjusting`] contextual state.
///
/// This state just walks towards the destination. If [`Moving::exact`] is true,
/// then it will perform small movement to ensure the `x` is as close as possible.
pub fn update_adjusting_context(
    context: &Context,
    state: &mut PlayerState,
    adjusting: Adjusting,
) -> Player {
    let moving = adjusting.moving;
    let cur_pos = state.last_known_pos.expect("in positional context");
    let (x_distance, x_direction) = moving.x_distance_direction_from(true, cur_pos);
    let is_intermediate = moving.is_destination_intermediate();

    match next_moving_lifecycle_with_axis(moving, cur_pos, MOVE_TIMEOUT, ChangeAxis::Both) {
        MovingLifecycle::Started(moving) => {
            // Check to perform a fall and returns to walk
            if !is_intermediate
                && state.config.teleport_key.is_none()
                && state.last_movement != Some(LastMovement::Falling)
                && state.is_stationary
                && x_distance >= ADJUSTING_MEDIUM_THRESHOLD
            {
                let (y_distance, y_direction) = moving.y_distance_direction_from(true, cur_pos);
                if y_direction < 0 && y_distance >= FALLING_THRESHOLD {
                    return Player::Falling {
                        moving: moving.timeout_started(false),
                        anchor: cur_pos,
                        timeout_on_complete: true,
                    };
                }
            }

            state.use_immediate_control_flow = true;
            state.last_movement = Some(LastMovement::Adjusting);

            Player::Adjusting(adjusting.moving(moving))
        }
        MovingLifecycle::Ended(moving) => {
            let _ = context.keys.send_up(KeyKind::Right);
            let _ = context.keys.send_up(KeyKind::Left);

            Player::Moving(moving.dest, moving.exact, moving.intermediates)
        }
        MovingLifecycle::Updated(mut moving) => {
            let mut adjusting = adjusting;

            if x_distance >= state.double_jump_threshold(is_intermediate) {
                state.use_immediate_control_flow = true;
                return Player::Moving(moving.dest, moving.exact, moving.intermediates);
            }

            if !moving.completed {
                let adjusting_started = adjusting.adjust_timeout.started;
                if adjusting_started {
                    // Do not allow timing out if adjusting is in-progress
                    moving = moving.timeout_current(moving.timeout.current.saturating_sub(1));
                }

                let should_adjust_medium =
                    !adjusting_started && x_distance >= ADJUSTING_MEDIUM_THRESHOLD;
                let should_adjust_short =
                    adjusting_started || (moving.exact && x_distance >= ADJUSTING_SHORT_THRESHOLD);
                let direction = match x_direction.cmp(&0) {
                    Ordering::Greater => {
                        Some((KeyKind::Right, KeyKind::Left, ActionKeyDirection::Right))
                    }
                    Ordering::Less => {
                        Some((KeyKind::Left, KeyKind::Right, ActionKeyDirection::Left))
                    }
                    _ => None,
                };

                match (should_adjust_medium, should_adjust_short, direction) {
                    (true, _, Some((down_key, up_key, dir))) => {
                        let _ = context.keys.send_up(up_key);
                        let _ = context.keys.send_down(down_key);
                        state.last_known_direction = dir;
                    }
                    (false, true, Some((down_key, up_key, dir))) => {
                        adjusting.update_adjusting(context, up_key, down_key);
                        state.last_known_direction = dir;
                    }
                    _ => {
                        let _ = context.keys.send_up(KeyKind::Left);
                        let _ = context.keys.send_up(KeyKind::Right);
                        moving = moving.completed(true);
                    }
                }
            }

            on_action_state(
                state,
                |state, action| on_player_action(context, state, action, moving),
                || {
                    if !moving.completed {
                        return Player::Adjusting(adjusting.moving(moving));
                    }

                    if moving.exact && x_distance >= ADJUSTING_SHORT_THRESHOLD {
                        // Exact adjusting incomplete
                        return Player::Adjusting(
                            adjusting.moving(moving.completed(false).timeout_current(0)),
                        );
                    }

                    Player::Adjusting(adjusting.moving(moving.timeout_current(MOVE_TIMEOUT)))
                },
            )
        }
    }
}

fn on_player_action(
    context: &Context,
    state: &PlayerState,
    action: PlayerAction,
    moving: Moving,
) -> Option<(Player, bool)> {
    const USE_KEY_Y_THRESHOLD: i32 = 2;

    let cur_pos = state.last_known_pos.unwrap();
    let (x_distance, _) = moving.x_distance_direction_from(false, cur_pos);
    let (y_distance, _) = moving.y_distance_direction_from(false, cur_pos);

    match action {
        PlayerAction::Key(PlayerActionKey {
            with: ActionKeyWith::DoubleJump,
            direction,
            ..
        }) => {
            if !moving.completed || y_distance > 0 {
                return None;
            }
            if matches!(direction, ActionKeyDirection::Any)
                || direction == state.last_known_direction
            {
                Some((
                    Player::DoubleJumping(DoubleJumping::new(
                        moving.timeout(Timeout::default()).completed(false),
                        true,
                        false,
                    )),
                    false,
                ))
            } else {
                Some((Player::UseKey(UseKey::from_action(action)), false))
            }
        }
        PlayerAction::Key(PlayerActionKey {
            with: ActionKeyWith::Any,
            ..
        }) => {
            if moving.completed && y_distance <= USE_KEY_Y_THRESHOLD {
                Some((Player::UseKey(UseKey::from_action(action)), false))
            } else {
                None
            }
        }
        PlayerAction::AutoMob(_) => {
            on_auto_mob_use_key_action(context, action, moving.pos, x_distance, y_distance)
        }
        PlayerAction::Key(PlayerActionKey {
            with: ActionKeyWith::Stationary,
            ..
        })
        | PlayerAction::SolveRune
        | PlayerAction::Move(_) => None,
        PlayerAction::PingPong(_) | PlayerAction::Panic(_) | PlayerAction::FamiliarsSwapping(_) => {
            unreachable!()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use mockall::predicate::eq;
    use opencv::core::Point;

    use super::*;
    use crate::{
        bridge::MockKeySender,
        player::{Player, PlayerState},
    };

    #[test]
    fn update_adjusting_context_started_falling() {
        let context = Context::new(None, None);
        let pos = Point { x: 0, y: 10 };
        let dest = Point { x: 10, y: 0 };
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);
        state.is_stationary = true;
        let adjusting = Adjusting::new(Moving::new(pos, dest, false, None));

        let player = update_adjusting_context(&context, &mut state, adjusting);

        assert!(matches!(
            player,
            Player::Falling {
                moving: _,
                anchor: _,
                timeout_on_complete: true
            }
        ));
        assert!(!state.use_immediate_control_flow);
        assert!(state.last_movement.is_none());
    }

    #[test]
    fn update_adjusting_context_started() {
        let context = Context::new(None, None);
        let pos = Point { x: 0, y: 0 };
        let dest = Point { x: 10, y: 0 };
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);
        state.is_stationary = true;
        let adjusting = Adjusting::new(Moving::new(pos, dest, false, None));

        let player = update_adjusting_context(&context, &mut state, adjusting);

        assert_matches!(player, Player::Adjusting(_));
        assert_matches!(state.last_movement, Some(LastMovement::Adjusting));
        assert!(state.use_immediate_control_flow);
    }

    #[test]
    fn update_adjusting_context_updated_performs_medium_adjustment_right() {
        let mut keys = MockKeySender::default();
        // Expect right to be pressed down and left to be released
        keys.expect_send_up()
            .with(eq(KeyKind::Left))
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_down()
            .with(eq(KeyKind::Right))
            .once()
            .returning(|_| Ok(()));

        let context = Context::new(Some(keys), None);

        let pos = Point { x: 0, y: 0 };
        let dest = Point { x: 5, y: 0 }; // x_distance = 5 (>= medium threshold = 3)
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);

        let moving = Moving::new(pos, dest, false, None).timeout_started(true);
        let adjusting = Adjusting::new(moving);

        let player = update_adjusting_context(&context, &mut state, adjusting);

        assert_matches!(player, Player::Adjusting(_));
        assert_eq!(state.last_known_direction, ActionKeyDirection::Right);
    }

    #[test]
    fn update_adjusting_context_updated_performs_medium_adjustment_left() {
        let mut keys = MockKeySender::default();
        keys.expect_send_up()
            .with(eq(KeyKind::Right))
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_down()
            .with(eq(KeyKind::Left))
            .once()
            .returning(|_| Ok(()));

        let context = Context::new(Some(keys), None);
        let pos = Point { x: 10, y: 0 };
        let dest = Point { x: 0, y: 0 }; // x_distance = 10
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);

        let moving = Moving::new(pos, dest, false, None).timeout_started(true);
        let adjusting = Adjusting::new(moving);

        let player = update_adjusting_context(&context, &mut state, adjusting);

        assert_matches!(player, Player::Adjusting(_));
        assert_eq!(state.last_known_direction, ActionKeyDirection::Left);
    }

    #[test]
    fn update_adjusting_context_updated_completes_when_no_direction_and_no_adjustment() {
        let mut keys = MockKeySender::default();
        keys.expect_send_up()
            .with(eq(KeyKind::Left))
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_up()
            .with(eq(KeyKind::Right))
            .once()
            .returning(|_| Ok(()));

        let context = Context::new(Some(keys), None);
        let pos = Point { x: 5, y: 0 };
        let dest = Point { x: 5, y: 0 }; // same position, no direction
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);

        let moving = Moving::new(pos, dest, false, None).timeout_started(true);
        let adjusting = Adjusting::new(moving);

        let player = update_adjusting_context(&context, &mut state, adjusting);

        assert_matches!(
            player,
            Player::Adjusting(Adjusting {
                moving: Moving {
                    completed: true,
                    ..
                },
                ..
            })
        );
    }

    #[test]
    fn update_adjusting_context_updated_short_adjustment_started() {
        let mut keys = MockKeySender::default();
        keys.expect_send_up()
            .with(eq(KeyKind::Left))
            .once()
            .returning(|_| Ok(()));
        keys.expect_send()
            .with(eq(KeyKind::Right))
            .once()
            .returning(|_| Ok(()));

        let context = Context::new(Some(keys), None);
        let pos = Point { x: 0, y: 0 };
        let dest = Point { x: 1, y: 0 }; // exact = true, x_distance = 1
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);

        let moving = Moving::new(pos, dest, true, None).timeout_started(true);
        let adjusting = Adjusting::new(moving);

        let player = update_adjusting_context(&context, &mut state, adjusting);

        assert_matches!(
            player,
            Player::Adjusting(Adjusting {
                adjust_timeout: Timeout { started: true, .. },
                ..
            })
        );
        assert_eq!(state.last_known_direction, ActionKeyDirection::Right);
    }

    #[test]
    fn update_adjusting_context_updated_timeout_freezes_when_adjusting_started() {
        let context = Context::new(None, None);
        let pos = Point { x: 0, y: 0 };
        let dest = Point { x: 1, y: 0 };
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);

        let moving = Moving::new(pos, dest, true, None)
            .timeout_current(3)
            .timeout_started(true);
        let mut adjusting = Adjusting::new(moving);
        adjusting.adjust_timeout = Timeout {
            current: 1,
            started: true,
            ..Default::default()
        };

        let player = update_adjusting_context(&context, &mut state, adjusting);

        assert_matches!(
            player,
            Player::Adjusting(Adjusting {
                moving: Moving {
                    timeout: Timeout { current: 3, .. }, // stay the same
                    ..
                },
                adjust_timeout: Timeout { current: 2, .. }
            })
        );
    }

    #[test]
    fn update_adjusting_context_updated_complted_exact_not_close_enough_keeps_adjusting() {
        let context = Context::new(None, None);
        let pos = Point { x: 0, y: 0 };
        let dest = Point { x: 1, y: 0 };
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);
        let moving = Moving::new(pos, dest, true, None)
            .completed(true)
            .timeout_current(4)
            .timeout_started(true);
        let adjusting = Adjusting::new(moving);

        let player = update_adjusting_context(&context, &mut state, adjusting);

        assert_matches!(
            player,
            Player::Adjusting(Adjusting {
                moving: Moving {
                    completed: false,
                    timeout: Timeout {
                        current: 0,
                        started: true,
                        ..
                    },
                    ..
                },
                ..
            })
        );
    }

    // TODO: add tests for on_action
}
