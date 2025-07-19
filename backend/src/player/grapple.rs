use super::{
    Player, PlayerAction, PlayerActionPingPong, PlayerState,
    actions::{on_action_state, on_auto_mob_use_key_action, on_ping_pong_double_jump_action},
    moving::Moving,
    state::LastMovement,
    timeout::{MovingLifecycle, next_moving_lifecycle_with_axis},
};
use crate::{
    context::Context,
    player::{MOVE_TIMEOUT, timeout::ChangeAxis},
};

/// Minimum y distance from the destination required to perform a grappling hook.
pub const GRAPPLING_THRESHOLD: i32 = 24;

/// Maximum y distance from the destination allowed to perform a grappling hook.
pub const GRAPPLING_MAX_THRESHOLD: i32 = 41;

/// Timeout for grappling.
const TIMEOUT: u32 = MOVE_TIMEOUT * 8;

/// Timeout after stopping grappling.
const STOPPING_TIMEOUT: u32 = MOVE_TIMEOUT + 3;

/// Maximum y distance allowed to stop grappling.
const STOPPING_THRESHOLD: i32 = 3;

/// Updates the [`Player::Grappling`] contextual state.
///
/// This state can only be transitioned via [`Player::Moving`] or [`Player::DoubleJumping`]
/// when the player has reached or close to the destination x-wise.
///
/// This state will use the Rope Lift skill.
pub fn update_grappling_context(
    context: &Context,
    state: &mut PlayerState,
    moving: Moving,
) -> Player {
    let key = state
        .config
        .grappling_key
        .expect("cannot transition if not set");
    let prev_pos = moving.pos;

    match next_moving_lifecycle_with_axis(
        moving,
        state.last_known_pos.expect("in positional context"),
        TIMEOUT,
        ChangeAxis::Vertical,
    ) {
        MovingLifecycle::Started(moving) => {
            state.last_movement = Some(LastMovement::Grappling);
            let _ = context.keys.send(key);
            Player::Grappling(moving)
        }
        MovingLifecycle::Ended(moving) => {
            Player::Moving(moving.dest, moving.exact, moving.intermediates)
        }
        MovingLifecycle::Updated(mut moving) => {
            let cur_pos = moving.pos;
            let (y_distance, y_direction) = moving.y_distance_direction_from(true, cur_pos);
            let x_changed = prev_pos.x != cur_pos.x;

            if moving.timeout.current >= MOVE_TIMEOUT && x_changed {
                // During double jump and grappling failed
                moving = moving.timeout_current(TIMEOUT).completed(true);
            }
            if !moving.completed {
                if y_direction <= 0 || y_distance <= stopping_threshold(state.velocity.1) {
                    let _ = context.keys.send(key);
                    moving = moving.completed(true);
                }
            } else if moving.timeout.current >= STOPPING_TIMEOUT {
                moving = moving.timeout_current(TIMEOUT);
            }

            on_action_state(
                state,
                |state, action| match action {
                    PlayerAction::AutoMob(_) => {
                        if !moving.completed {
                            return None;
                        }
                        if moving.is_destination_intermediate() {
                            return Some((
                                Player::Moving(moving.dest, moving.exact, moving.intermediates),
                                false,
                            ));
                        }
                        if state.config.teleport_key.is_some() && !moving.completed {
                            return None;
                        }

                        let (x_distance, _) = moving.x_distance_direction_from(false, cur_pos);
                        let (y_distance, _) = moving.y_distance_direction_from(false, cur_pos);
                        on_auto_mob_use_key_action(context, action, cur_pos, x_distance, y_distance)
                    }
                    PlayerAction::PingPong(PlayerActionPingPong {
                        bound, direction, ..
                    }) => {
                        if cur_pos.y >= bound.y
                            && context.rng.random_perlin_bool(
                                cur_pos.x,
                                cur_pos.y,
                                context.tick,
                                0.7,
                            )
                        {
                            Some(on_ping_pong_double_jump_action(
                                context, cur_pos, bound, direction,
                            ))
                        } else {
                            None
                        }
                    }
                    PlayerAction::Key(_) | PlayerAction::Move(_) | PlayerAction::SolveRune => None,
                    PlayerAction::Panic(_) | PlayerAction::FamiliarsSwapping(_) => unreachable!(),
                },
                || Player::Grappling(moving),
            )
        }
    }
}

/// Converts vertical velocity to a stopping threshold.
#[inline]
fn stopping_threshold(velocity: f32) -> i32 {
    (STOPPING_THRESHOLD as f32 + 1.07 * velocity).round() as i32
}

#[cfg(test)]
mod tests {
    use mockall::predicate::eq;
    use opencv::core::Point;
    use platforms::windows::KeyKind;

    use super::*;
    use crate::bridge::MockKeySender;

    const START_POS: Point = Point { x: 100, y: 100 };
    const END_POS: Point = Point { x: 100, y: 200 };

    fn mock_state_with_grapple(pos: Point) -> PlayerState {
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);
        state.config.grappling_key = Some(KeyKind::Space);
        state
    }

    fn mock_moving(pos: Point) -> Moving {
        Moving::new(pos, pos, false, None)
    }

    #[test]
    fn update_grappling_context_started() {
        let mut state = mock_state_with_grapple(END_POS);
        let moving = mock_moving(START_POS);
        let mut keys = MockKeySender::new();
        keys.expect_send()
            .once()
            .with(eq(KeyKind::Space))
            .returning(|_| Ok(()));
        let context = Context::new(Some(keys), None);

        let result = update_grappling_context(&context, &mut state, moving);

        match result {
            Player::Grappling(m) => {
                assert_eq!(m.pos, END_POS);
                assert_eq!(state.last_movement, Some(LastMovement::Grappling));
            }
            _ => panic!("Expected Player::Grappling"),
        }
    }

    #[test]
    fn update_grappling_context_updated_timeout_and_x_change() {
        let mut state = mock_state_with_grapple(START_POS);
        let context = Context::new(None, None);
        let mut moving = mock_moving(Point::new(START_POS.x + 10, START_POS.y)); // x changed
        moving.timeout.current = MOVE_TIMEOUT;
        moving.timeout.started = true;

        let result = update_grappling_context(&context, &mut state, moving);

        match result {
            Player::Grappling(m) => {
                println!("{m:?}");
                assert!(m.completed);
            }
            _ => panic!("Expected Player::Grappling"),
        }
    }

    #[test]
    fn update_grappling_context_updated_auto_complete_on_stopping_threshold() {
        let mut keys = MockKeySender::new();
        keys.expect_send()
            .once()
            .with(eq(KeyKind::Space))
            .returning(|_| Ok(()));
        let context = Context::new(Some(keys), None);
        let mut state = mock_state_with_grapple(Point::new(100, 103)); // close enough
        let mut moving = mock_moving(Point::new(100, 100));
        moving.timeout.started = true;

        let result = update_grappling_context(&context, &mut state, moving);

        match result {
            Player::Grappling(m) => {
                assert!(m.completed);
            }
            _ => panic!("Expected Player::Grappling"),
        }
    }

    // TODO: Add tests for on_action
}
