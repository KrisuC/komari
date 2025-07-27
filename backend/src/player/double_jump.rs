use std::cmp::Ordering;

use log::debug;
use opencv::core::{Point, Rect};

use super::{
    PingPongDirection, Player, PlayerAction, PlayerActionKey, PlayerState,
    actions::{PlayerActionPingPong, on_action_state, on_auto_mob_use_key_action},
    moving::Moving,
    timeout::{
        Lifecycle, MovingLifecycle, next_moving_lifecycle_with_axis, next_timeout_lifecycle,
    },
    up_jump::UpJumping,
    use_key::UseKey,
};
use crate::{
    ActionKeyDirection, ActionKeyWith,
    bridge::KeyKind,
    context::Context,
    player::{
        moving::MOVE_TIMEOUT,
        state::LastMovement,
        timeout::{ChangeAxis, Timeout},
    },
};

/// Minimum x distance from the destination required to perform a double jump.
pub const DOUBLE_JUMP_THRESHOLD: i32 = 25;

/// Minimum x distance from the destination required to perform a double jump in auto mobbing.
pub const DOUBLE_JUMP_AUTO_MOB_THRESHOLD: i32 = 15;

/// Minimum x distance from the destination required to transition to [`Player::UseKey`].
const USE_KEY_X_THRESHOLD: i32 = DOUBLE_JUMP_THRESHOLD;

/// Minimum y distance from the destination required to transition to [`Player::UseKey`].
const USE_KEY_Y_THRESHOLD: i32 = 10;

/// Maximum number of ticks before timing out.
const TIMEOUT: u32 = MOVE_TIMEOUT;

const TIMEOUT_FORCED: u32 = MOVE_TIMEOUT + 3;

/// Number of ticks to wait after a double jump.
///
/// A heuristic to mostly avoid mid-air jump keys sending. The current approach of using velocity
/// does not send much keys after double jumped, but only few are sent mid-air.
const COOLDOWN_TIMEOUT: u32 = MOVE_TIMEOUT;

/// Minimum x distance from the destination required to transition to [`Player::Grappling`].
const GRAPPLING_THRESHOLD: i32 = 4;

/// Minimum x velocity to be considered as double jumped.
const X_VELOCITY_THRESHOLD: f32 = 1.0;

/// Maximum x velocity allowed to be considered as near stationary.
const X_NEAR_STATIONARY_VELOCITY_THRESHOLD: f32 = 0.75;

/// Maximum y velocity allowed to be considered as near stationary.
const Y_NEAR_STATIONARY_VELOCITY_THRESHOLD: f32 = 0.4;

/// Minimium y distance required to perform a fall and then double jump.
const FALLING_THRESHOLD: i32 = 8;

/// Minimum y distance required from the middle y of ping pong bound to allow randomization.
const PING_PONG_IGNORE_RANDOMIZE_Y_THRESHOLD: i32 = 9;

#[derive(Copy, Clone, Debug)]
pub struct DoubleJumping {
    pub moving: Moving,
    /// Whether to force a double jump even when the player current position is already close to
    /// the destination.
    pub forced: bool,
    /// Whether to wait for the player is about to become stationary before sending jump keys.
    require_near_stationary: bool,
    /// Timeout for between double jump cooldown.
    cooldown_timeout: Timeout,
}

impl DoubleJumping {
    pub fn new(moving: Moving, forced: bool, require_stationary: bool) -> Self {
        Self {
            moving,
            forced,
            require_near_stationary: require_stationary,
            cooldown_timeout: Timeout::default(),
        }
    }

    #[inline]
    fn moving(self, moving: Moving) -> DoubleJumping {
        DoubleJumping { moving, ..self }
    }

    #[inline]
    fn update_jump_cooldown(&mut self) {
        self.cooldown_timeout =
            match next_timeout_lifecycle(self.cooldown_timeout, COOLDOWN_TIMEOUT) {
                Lifecycle::Started(timeout) => timeout,
                Lifecycle::Ended => Timeout::default(),
                Lifecycle::Updated(timeout) => timeout,
            };
    }
}

/// Updates the [`Player::DoubleJumping`] contextual state.
///
/// This state continues to double jump as long as the distance x-wise is still
/// `>= DOUBLE_JUMP_THRESHOLD`. Or when [`DoubleJumping::forced`], this state will attempt
/// a single double jump. When [`DoubleJumping::require_stationary`], this state will wait for
/// the player to be stationary before double jumping.
///
/// [`DoubleJumping::forced`] is currently true when it is transitioned
/// from [`Player::Idle`], [`Player::Moving`], [`Player::Adjusting`], and
/// [`Player::UseKey`] with [`PlayerState::last_known_direction`] matches the
/// [`PlayerAction::Key`] direction.
///
/// [`DoubleJumping::require_stationary`] is currently true when it is transitioned
/// from [`Player::Idle`] and [`Player::UseKey`] with [`PlayerState::last_known_direction`] matches
/// the [`PlayerAction::Key`] direction.
pub fn update_double_jumping_context(
    context: &Context,
    state: &mut PlayerState,
    double_jumping: DoubleJumping,
) -> Player {
    let moving = double_jumping.moving;
    let ignore_grappling = double_jumping.forced || state.should_disable_grappling();
    let is_intermediate = moving.is_destination_intermediate();
    let timeout = if double_jumping.forced {
        TIMEOUT_FORCED
    } else {
        TIMEOUT
    };
    let axis = if double_jumping.forced {
        // This ensures it won't double jump forever when jumping towards either
        // edges of the map.
        ChangeAxis::Horizontal
    } else {
        ChangeAxis::Both
    };

    match next_moving_lifecycle_with_axis(
        moving,
        state.last_known_pos.expect("in positional context"),
        timeout,
        axis,
    ) {
        MovingLifecycle::Started(moving) => {
            // Check to perform a fall and returns to double jump
            if !is_intermediate
                && !double_jumping.forced
                && state.config.teleport_key.is_none()
                && state.last_movement != Some(LastMovement::Falling)
                && state.is_stationary
            {
                let (y_distance, y_direction) = moving.y_distance_direction_from(true, moving.pos);
                if y_direction < 0 && y_distance >= FALLING_THRESHOLD {
                    return Player::Falling {
                        moving: moving.timeout_started(false),
                        anchor: moving.pos,
                        timeout_on_complete: true,
                    };
                }
            }

            // Stall until near stationary by resetting started
            if double_jumping.require_near_stationary {
                let (x_velocity, y_velocity) = state.velocity;
                if x_velocity > X_NEAR_STATIONARY_VELOCITY_THRESHOLD
                    || y_velocity > Y_NEAR_STATIONARY_VELOCITY_THRESHOLD
                {
                    return Player::DoubleJumping(
                        double_jumping.moving(moving.timeout_started(false)),
                    );
                }
            }

            state.use_immediate_control_flow = true;
            state.last_movement = Some(LastMovement::DoubleJumping);

            Player::DoubleJumping(double_jumping.moving(moving))
        }
        MovingLifecycle::Ended(moving) => {
            let _ = context.input.send_key_up(KeyKind::Right);
            let _ = context.input.send_key_up(KeyKind::Left);

            Player::Moving(moving.dest, moving.exact, moving.intermediates)
        }
        MovingLifecycle::Updated(mut moving) => {
            let (x_distance, x_direction) = moving.x_distance_direction_from(true, moving.pos);
            let mut double_jumping = double_jumping;

            if !moving.completed {
                if !double_jumping.forced || state.config.teleport_key.is_some() {
                    let option = match x_direction.cmp(&0) {
                        Ordering::Greater => {
                            Some((KeyKind::Right, KeyKind::Left, ActionKeyDirection::Right))
                        }
                        Ordering::Less => {
                            Some((KeyKind::Left, KeyKind::Right, ActionKeyDirection::Left))
                        }
                        _ => {
                            // Mage teleportation requires a direction
                            if state.config.teleport_key.is_some() {
                                get_mage_teleport_direction(state)
                            } else {
                                None
                            }
                        }
                    };
                    if let Some((key_down, key_up, direction)) = option {
                        let _ = context.input.send_key_down(key_down);
                        let _ = context.input.send_key_up(key_up);
                        state.last_known_direction = direction;
                    }
                }

                let can_continue = !double_jumping.forced
                    && x_distance >= state.double_jump_threshold(is_intermediate);
                let can_press = double_jumping.forced && state.velocity.0 <= X_VELOCITY_THRESHOLD;
                if can_continue || can_press {
                    if !double_jumping.cooldown_timeout.started
                        && state.velocity.0 <= X_VELOCITY_THRESHOLD
                    {
                        let _ = context
                            .input
                            .send_key(state.config.teleport_key.unwrap_or(state.config.jump_key));
                    } else {
                        double_jumping.update_jump_cooldown();
                    }
                } else {
                    let _ = context.input.send_key_up(KeyKind::Right);
                    let _ = context.input.send_key_up(KeyKind::Left);
                    moving = moving.completed(true);
                }
            }

            on_action_state(
                state,
                |state, action| {
                    on_player_action(
                        context,
                        state,
                        action,
                        moving,
                        double_jumping.forced,
                        state.velocity.0 > X_VELOCITY_THRESHOLD,
                    )
                },
                || {
                    if !ignore_grappling && moving.completed && x_distance <= GRAPPLING_THRESHOLD {
                        let (_, y_direction) = moving.y_distance_direction_from(true, moving.pos);
                        if y_direction > 0 {
                            debug!(target: "player", "performs grappling on double jump");
                            return Player::Grappling(
                                moving.completed(false).timeout(Timeout::default()),
                            );
                        }
                    }

                    if moving.completed {
                        Player::DoubleJumping(
                            double_jumping.moving(moving.timeout_current(TIMEOUT)),
                        )
                    } else {
                        Player::DoubleJumping(double_jumping.moving(moving))
                    }
                },
            )
        }
    }
}

/// Handles [`PlayerAction`] during double jump.
///
/// It currently handles action for auto mob and a key action with [`ActionKeyWith::Any`] or
/// [`ActionKeyWith::DoubleJump`]. For auto mob, the same handling logics is reused. For the other,
/// it will try to transition to [`Player::UseKey`] when the player is close enough.
fn on_player_action(
    context: &Context,
    state: &PlayerState,
    action: PlayerAction,
    moving: Moving,
    forced: bool,
    double_jumped_or_flying: bool,
) -> Option<(Player, bool)> {
    let cur_pos = state.last_known_pos.unwrap();
    let (x_distance, _) = moving.x_distance_direction_from(false, cur_pos);
    let (y_distance, _) = moving.y_distance_direction_from(false, cur_pos);

    match action {
        PlayerAction::PingPong(PlayerActionPingPong {
            bound, direction, ..
        }) => on_ping_pong_use_key_action(
            context,
            action,
            cur_pos,
            bound,
            direction,
            double_jumped_or_flying,
            state.config.grappling_key.is_some(),
        ),
        PlayerAction::AutoMob(_) => {
            on_auto_mob_use_key_action(context, action, moving.pos, x_distance, y_distance)
        }
        PlayerAction::Key(PlayerActionKey {
            with: ActionKeyWith::DoubleJump | ActionKeyWith::Any,
            ..
        }) => {
            if !moving.completed {
                return None;
            }
            // Ignore proximity check when it is forced to double jumped as this indicates the
            // player is already near the destination.
            if forced
                || (!moving.exact
                    && x_distance <= USE_KEY_X_THRESHOLD
                    && y_distance <= USE_KEY_Y_THRESHOLD)
            {
                Some((Player::UseKey(UseKey::from_action(action)), false))
            } else {
                None
            }
        }
        PlayerAction::Key(PlayerActionKey {
            with: ActionKeyWith::Stationary,
            ..
        })
        | PlayerAction::SolveRune
        | PlayerAction::Move { .. } => None,
        PlayerAction::Panic(_) | PlayerAction::FamiliarsSwapping(_) => unreachable!(),
    }
}

/// Handles ping pong action during double jump.
///
/// This function checks for specific conditions to decide whether to:
/// - Transition to [`Player::Idle`] when player hits horizontal bounds
/// - If the player has double jumped or already flying:
///   - Transition to [`Player::Falling`] or [`Player::UpJumping`] with a chance to simulate vertical movement
///   - Transition to [`Player::UseKey`] otherwise
#[inline]
fn on_ping_pong_use_key_action(
    context: &Context,
    action: PlayerAction,
    cur_pos: Point,
    bound: Rect,
    direction: PingPongDirection,
    double_jumped: bool,
    has_grappling: bool,
) -> Option<(Player, bool)> {
    let hit_x_bound_edge = match direction {
        PingPongDirection::Left => cur_pos.x - bound.x <= 0,
        PingPongDirection::Right => cur_pos.x - bound.x - bound.width >= 0,
    };
    if hit_x_bound_edge {
        return Some((Player::Idle, true));
    }
    if !double_jumped {
        return None;
    }

    let _ = context.input.send_key_up(KeyKind::Left);
    let _ = context.input.send_key_up(KeyKind::Right);
    let bound_y_max = bound.y + bound.height;
    let bound_y_mid = bound.y + bound.height / 2;

    let allow_randomize = (cur_pos.y - bound_y_mid).abs() >= PING_PONG_IGNORE_RANDOMIZE_Y_THRESHOLD;
    let upward_bias = allow_randomize && cur_pos.y < bound_y_mid;
    let downward_bias = allow_randomize && cur_pos.y > bound_y_mid;
    let should_upward = upward_bias
        && context
            .rng
            .random_perlin_bool(cur_pos.x, cur_pos.y, context.tick, 0.35);
    let should_downward = downward_bias
        && context
            .rng
            .random_perlin_bool(cur_pos.x, cur_pos.y, context.tick + 100, 0.25);

    if cur_pos.y < bound.y || should_upward {
        let moving = Moving::new(
            cur_pos,
            Point::new(cur_pos.x, bound.y + bound.height),
            false,
            None,
        );
        let next = if has_grappling {
            Player::Grappling(moving)
        } else {
            Player::UpJumping(UpJumping::new(moving))
        };
        return Some((next, false));
    }

    if cur_pos.y > bound_y_max || should_downward {
        return Some((
            Player::Falling {
                moving: Moving::new(cur_pos, Point::new(cur_pos.x, bound.y), false, None),
                anchor: cur_pos,
                timeout_on_complete: true,
            },
            false,
        ));
    }

    Some((Player::UseKey(UseKey::from_action(action)), false))
}

/// Gets the mage teleport direction when the player is already at destination.
fn get_mage_teleport_direction(
    state: &PlayerState,
) -> Option<(KeyKind, KeyKind, ActionKeyDirection)> {
    // FIXME: Currently, PlayerActionKey with double jump + has position + has direction:
    //  1. Double jump near proximity
    //  2. Transition to UseKey and update direction
    //  3. Transition back to double jump
    //  4. Use last_known_direction to double jump
    //
    // This will cause mage to teleport to the opposite direction of destination, which is not
    // desired. The desired behavior would be to use skill near the destination in the direction
    // specified by PlayerActionKey. HOW TO FIX?
    match state.last_known_direction {
        // Clueless
        ActionKeyDirection::Any => None,
        ActionKeyDirection::Right => {
            Some((KeyKind::Right, KeyKind::Left, ActionKeyDirection::Right))
        }
        ActionKeyDirection::Left => Some((KeyKind::Left, KeyKind::Right, ActionKeyDirection::Left)),
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use anyhow::Ok;
    use opencv::core::{Point, Rect};

    use super::{on_ping_pong_use_key_action, update_double_jumping_context};
    use crate::{
        ActionKeyDirection,
        bridge::{KeyKind, MockInput},
        context::Context,
        player::{
            PingPongDirection, Player, PlayerAction, PlayerActionPingPong,
            double_jump::DoubleJumping,
            moving::Moving,
            state::{LastMovement, PlayerState},
            timeout::Timeout,
        },
    };

    #[test]
    fn update_double_jumping_context_started() {
        let pos = Point::new(0, 0);
        let dest = Point::new(30, 0);
        let context = Context::new(None, None);
        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);
        let moving = Moving::new(pos, dest, false, None);
        let double_jump = DoubleJumping::new(moving, false, false);

        let player = update_double_jumping_context(&context, &mut state, double_jump);
        assert_matches!(player, Player::DoubleJumping(_));
        assert!(state.use_immediate_control_flow);
        assert_eq!(state.last_movement, Some(LastMovement::DoubleJumping));
    }

    #[test]
    fn update_double_jumping_context_started_falls_if_dest_above_and_close() {
        let pos = Point::new(0, 10);
        let dest = Point::new(0, 0);
        let context = Context::new(None, None);
        let mut state = PlayerState::default();
        state.is_stationary = true;
        state.last_known_pos = Some(pos);
        let moving = Moving::new(pos, dest, false, None);
        let double_jump = DoubleJumping::new(moving, false, false);

        let player = update_double_jumping_context(&context, &mut state, double_jump);
        assert_matches!(
            player,
            Player::Falling {
                moving: _,
                anchor: _,
                timeout_on_complete: true
            }
        );
    }

    #[test]
    fn update_double_jumping_context_updated_correct_direction() {
        let pos = Point::new(100, 50);
        let dest = Point::new(50, 50); // Move to the left
        let moving = Moving {
            pos,
            dest,
            timeout: Timeout {
                started: true,
                ..Timeout::default()
            },
            ..Default::default()
        };
        let jumping = DoubleJumping::new(moving, false, false);

        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);
        state.config.jump_key = KeyKind::Space;

        let mut keys = MockInput::new();
        keys.expect_send_key_down()
            .withf(|k| matches!(k, KeyKind::Left))
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_key_up()
            .withf(|k| matches!(k, KeyKind::Right))
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_key()
            .withf(|k| matches!(k, KeyKind::Space))
            .once()
            .returning(|_| Ok(()));
        let context = Context::new(Some(keys), None);

        update_double_jumping_context(&context, &mut state, jumping);
    }

    #[test]
    fn update_double_jumping_context_updated_forced_presses_only_jump_key() {
        let mut keys = MockInput::new();
        keys.expect_send_key()
            .withf(|&key| key == KeyKind::Space) // or use jump_key if needed
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_key_down().never();
        keys.expect_send_key_up().never();
        let context = Context::new(Some(keys), None);

        let mut state = PlayerState::default();
        state.last_known_pos = Some(Point::new(0, 0));
        state.velocity = (0.5, 0.0); // Low enough for X_VELOCITY_THRESHOLD
        state.config.jump_key = KeyKind::Space;

        let moving =
            Moving::new(Point::new(0, 0), Point::new(0, 0), true, None).timeout_started(true);
        let double_jump = DoubleJumping::new(moving, true, false); // forced=true

        let player = update_double_jumping_context(&context, &mut state, double_jump);

        assert_matches!(player, Player::DoubleJumping(_));
    }

    #[test]
    fn update_double_jumping_context_started_requires_stationary_and_stalls_if_velocity_high() {
        let context = Context::new(None, None);
        let mut state = PlayerState::default();
        state.last_known_pos = Some(Point::new(0, 0));
        state.velocity = (1.5, 0.5); // Too fast

        let moving = Moving::new(Point::new(0, 0), Point::new(50, 0), false, None);
        let double_jump = DoubleJumping::new(moving, false, true); // require_near_stationary = true

        let player = update_double_jumping_context(&context, &mut state, double_jump);

        assert_matches!(
            player,
            Player::DoubleJumping(DoubleJumping {
                moving: Moving {
                    timeout: Timeout { started: false, .. }, // stalls
                    ..
                },
                ..
            })
        );
        assert!(
            !state.use_immediate_control_flow,
            "Should not trigger flow until stationary"
        );
    }

    #[test]
    fn update_double_jumping_context_updated_mage_requires_direction_even_when_x_direction_zero() {
        let pos = Point::new(100, 50);
        let dest = pos; // Same x => x_direction == 0
        let moving = Moving {
            pos,
            dest,
            timeout: Timeout {
                started: true,
                ..Timeout::default()
            },
            ..Default::default()
        };
        let jumping = DoubleJumping::new(moving, true, false);

        let mut state = PlayerState::default();
        state.last_known_pos = Some(pos);
        state.last_known_direction = ActionKeyDirection::Right;
        state.config.teleport_key = Some(KeyKind::Shift); // Mage

        let mut keys = MockInput::new();
        keys.expect_send_key_down()
            .withf(|k| matches!(k, KeyKind::Right)) // Must still send right
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_key_up()
            .withf(|k| matches!(k, KeyKind::Left))
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_key()
            .withf(|k| matches!(k, KeyKind::Shift)) // Teleport key used, not jump
            .once()
            .returning(|_| Ok(()));
        let context = Context::new(Some(keys), None);

        update_double_jumping_context(&context, &mut state, jumping);
    }

    #[test]
    fn ping_pong_hits_left_bound_transitions_to_idle() {
        let cur_pos = Point::new(10, 100);
        let bound = Rect::new(20, 90, 40, 20); // left = 20
        let action = PlayerAction::PingPong(PlayerActionPingPong {
            bound,
            direction: PingPongDirection::Left,
            ..Default::default()
        });

        let context = Context::new(None, None);
        let result = on_ping_pong_use_key_action(
            &context,
            action,
            cur_pos,
            bound,
            PingPongDirection::Left,
            true,
            false,
        );
        assert_matches!(result, Some((Player::Idle, true)));
    }

    #[test]
    fn ping_pong_before_double_jump_returns_none() {
        let cur_pos = Point::new(30, 100);
        let bound = Rect::new(20, 90, 40, 20);
        let action = PlayerAction::PingPong(PlayerActionPingPong {
            bound,
            direction: PingPongDirection::Right,
            ..Default::default()
        });

        let context = Context::new(None, None);
        let result = on_ping_pong_use_key_action(
            &context,
            action,
            cur_pos,
            bound,
            PingPongDirection::Right,
            false, // hasn't double jumped
            true,
        );
        assert_matches!(result, None);
    }

    #[test]
    fn ping_pong_transition_to_upjumping_or_grappling() {
        let cur_pos = Point::new(30, 79); // below y
        let bound = Rect::new(20, 80, 40, 20);
        let action = PlayerAction::PingPong(PlayerActionPingPong {
            bound,
            direction: PingPongDirection::Right,
            ..Default::default()
        });

        let mut keys = MockInput::new();
        keys.expect_send_key_up().returning(|_| Ok(()));
        let context = Context::new(Some(keys), None);
        let result = on_ping_pong_use_key_action(
            &context,
            action,
            cur_pos,
            bound,
            PingPongDirection::Right,
            true,
            false, // no grappling
        );
        assert_matches!(result, Some((Player::UpJumping(_), false)));

        let result_with_grappling = on_ping_pong_use_key_action(
            &context,
            action,
            cur_pos,
            bound,
            PingPongDirection::Right,
            true,
            true,
        );
        assert_matches!(result_with_grappling, Some((Player::Grappling(_), false)));
    }

    #[test]
    fn ping_pong_transition_to_falling() {
        let cur_pos = Point::new(30, 101); // above y
        let bound = Rect::new(20, 80, 40, 20);
        let action = PlayerAction::PingPong(PlayerActionPingPong {
            bound,
            direction: PingPongDirection::Right,
            ..Default::default()
        });

        let mut keys = MockInput::new();
        keys.expect_send_key_up().returning(|_| Ok(()));
        let context = Context::new(Some(keys), None);
        let result = on_ping_pong_use_key_action(
            &context,
            action,
            cur_pos,
            bound,
            PingPongDirection::Right,
            true,
            false,
        );
        matches!(
            result,
            Some((
                Player::Falling {
                    moving: _,
                    anchor: _,
                    timeout_on_complete: true
                },
                false
            ))
        );
    }

    // TODO: Add tests for player action
}
