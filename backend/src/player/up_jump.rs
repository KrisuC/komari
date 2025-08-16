use super::{
    Key, PingPong, Player, PlayerState,
    actions::on_ping_pong_double_jump_action,
    moving::Moving,
    timeout::{MovingLifecycle, next_moving_lifecycle_with_axis},
    use_key::UseKey,
};
use crate::{
    ActionKeyWith,
    bridge::KeyKind,
    context::Context,
    minimap::Minimap,
    player::{
        MOVE_TIMEOUT, PlayerAction,
        actions::{on_action, on_auto_mob_use_key_action},
        state::LastMovement,
        timeout::ChangeAxis,
    },
};

/// Number of ticks to wait before spamming jump key.
const SPAM_DELAY: u32 = 7;

/// Number of ticks to wait before spamming jump key for lesser travel distance.
const SOFT_SPAM_DELAY: u32 = 12;

const TIMEOUT: u32 = MOVE_TIMEOUT + 3;

/// Player's `y` velocity to be considered as up jumped.
const UP_JUMPED_Y_VELOCITY_THRESHOLD: f32 = 1.3;

/// Player's `x` velocity to be considered as near stationary.
const X_NEAR_STATIONARY_THRESHOLD: f32 = 0.28;

/// Minimum distance required to perform an up jump using teleport key with jump.
const TELEPORT_WITH_JUMP_THRESHOLD: i32 = 20;

/// Minimum distance required to perform an up jump and then teleport.
const UP_JUMP_AND_TELEPORT_THRESHOLD: i32 = 23;

const SOFT_UP_JUMP_THRESHOLD: i32 = 16;

#[derive(Debug, Clone, Copy)]
enum UpJumpingKind {
    Mage,
    UpArrow,
    JumpKey,
    SpecificKey,
}

// TODO: Reorganize states
#[derive(Debug, Clone, Copy)]
pub struct UpJumping {
    pub moving: Moving,
    /// Number of ticks to wait before sending jump key(s).
    spam_delay: u32,
    /// Whether auto-mobbing should wait for up jump completion in non-intermediate destination.
    ///
    /// This is false initially but randomized on start lifecycle.
    auto_mob_wait_completion: bool,
    mage_did_up_jump: bool,
    mage_use_teleport_after_up_jump: bool,
}

impl UpJumping {
    // TODO: Compute `UpJumpingKind` upon construction?
    pub fn new(moving: Moving) -> Self {
        let (y_distance, _) = moving.y_distance_direction_from(true, moving.pos);
        let spam_delay = if y_distance <= SOFT_UP_JUMP_THRESHOLD {
            SOFT_SPAM_DELAY
        } else {
            SPAM_DELAY
        };

        Self {
            moving,
            spam_delay,
            auto_mob_wait_completion: false,
            mage_did_up_jump: false,
            mage_use_teleport_after_up_jump: false,
        }
    }

    #[inline]
    fn moving(self, moving: Moving) -> UpJumping {
        UpJumping { moving, ..self }
    }

    #[inline]
    fn auto_mob_wait_completion(self, auto_mob_wait_completion: bool) -> UpJumping {
        UpJumping {
            auto_mob_wait_completion,
            ..self
        }
    }
}

/// Updates the [`Player::UpJumping`] contextual state
///
/// This state can only be transitioned via [`Player::Moving`] when the
/// player has reached the destination x-wise. Before performing an up jump, it will check for
/// stationary state and whether the player is currently near a portal. If the player is near
/// a portal, this action is aborted. The up jump action is made to be adapted for various classes
/// that has different up jump key combination.
pub fn update_up_jumping_context(
    context: &Context,
    state: &mut PlayerState,
    up_jumping: UpJumping,
) -> Player {
    let up_jump_key = state.config.upjump_key;
    let jump_key = state.config.jump_key;
    let teleport_key = state.config.teleport_key;
    let has_teleport_key = teleport_key.is_some();
    let kind = up_jumping_kind(up_jump_key, has_teleport_key);

    match next_moving_lifecycle_with_axis(
        up_jumping.moving,
        state.last_known_pos.expect("in positional context"),
        TIMEOUT,
        ChangeAxis::Vertical,
    ) {
        MovingLifecycle::Started(moving) => {
            // Stall until near stationary
            if state.velocity.0 > X_NEAR_STATIONARY_THRESHOLD {
                return Player::UpJumping(up_jumping.moving(moving.timeout_started(false)));
            }

            if let Minimap::Idle(idle) = context.minimap
                && idle.is_position_inside_portal(moving.pos)
            {
                state.clear_action_completed();
                return Player::Idle;
            }
            state.last_movement = Some(LastMovement::UpJumping);

            let mut up_jumping = up_jumping;
            let (y_distance, _) = moving.y_distance_direction_from(true, moving.pos);
            match kind {
                UpJumpingKind::Mage => {
                    up_jumping.mage_use_teleport_after_up_jump =
                        y_distance >= UP_JUMP_AND_TELEPORT_THRESHOLD;

                    let _ = context.input.send_key_down(KeyKind::Up);
                    if y_distance >= TELEPORT_WITH_JUMP_THRESHOLD {
                        // TODO: If a dedicated up jump key is set, a jump should not be performed.
                        // TODO: But what mage class has a dedicated up jump that is not
                        // TODO: a normal jump key/space?
                        let _ = context.input.send_key(jump_key);
                    }
                }
                UpJumpingKind::UpArrow => {
                    let _ = context.input.send_key(jump_key);
                }
                UpJumpingKind::JumpKey => {
                    let _ = context.input.send_key_down(KeyKind::Up);
                    let _ = context.input.send_key(jump_key);
                }
                UpJumpingKind::SpecificKey => {
                    let _ = context.input.send_key_down(KeyKind::Up);
                }
            }

            // TODO: Should be fine to not check auto-mob action only?
            Player::UpJumping(
                up_jumping
                    .moving(moving)
                    .auto_mob_wait_completion(context.rng.random_bool(0.5)),
            )
        }
        MovingLifecycle::Ended(moving) => {
            let _ = context.input.send_key_up(KeyKind::Up);
            Player::Moving(moving.dest, moving.exact, moving.intermediates)
        }
        MovingLifecycle::Updated(mut moving) => {
            let mut up_jumping = up_jumping;
            let cur_pos = moving.pos;
            let (y_distance, y_direction) = moving.y_distance_direction_from(true, moving.pos);

            if moving.completed {
                let _ = context.input.send_key_up(KeyKind::Up);
            } else {
                match kind {
                    UpJumpingKind::Mage => {
                        perform_mage_up_jump(
                            context,
                            state,
                            &mut moving,
                            &mut up_jumping,
                            y_distance,
                        );
                    }
                    UpJumpingKind::UpArrow | UpJumpingKind::JumpKey => {
                        if state.velocity.1 <= UP_JUMPED_Y_VELOCITY_THRESHOLD {
                            // Spam jump/up arrow key until the player y changes
                            // above a threshold as sending jump key twice
                            // doesn't work.
                            if moving.timeout.total >= up_jumping.spam_delay {
                                if matches!(kind, UpJumpingKind::UpArrow) {
                                    let _ = context.input.send_key(KeyKind::Up);
                                } else {
                                    let _ = context.input.send_key(jump_key);
                                }
                            }
                        } else {
                            moving = moving.completed(true);
                        }
                    }
                    UpJumpingKind::SpecificKey => {
                        let _ = context.input.send_key(up_jump_key.expect("has up jum key"));
                        moving = moving.completed(true);
                    }
                }
            }

            on_action(
                state,
                |action| match action {
                    PlayerAction::AutoMob(_) => {
                        if moving.completed
                            && moving.is_destination_intermediate()
                            && y_direction <= 0
                        {
                            let _ = context.input.send_key_up(KeyKind::Up);
                            return Some((
                                Player::Moving(moving.dest, moving.exact, moving.intermediates),
                                false,
                            ));
                        }
                        if up_jumping.auto_mob_wait_completion && !moving.completed {
                            return None;
                        }
                        let (x_distance, _) = moving.x_distance_direction_from(false, cur_pos);
                        let (y_distance, _) = moving.y_distance_direction_from(false, cur_pos);
                        on_auto_mob_use_key_action(context, action, cur_pos, x_distance, y_distance)
                    }
                    PlayerAction::Key(Key {
                        with: ActionKeyWith::Any,
                        ..
                    }) => {
                        if !moving.completed || y_direction > 0 {
                            None
                        } else {
                            Some((Player::UseKey(UseKey::from_action(action)), false))
                        }
                    }
                    PlayerAction::PingPong(PingPong {
                        bound, direction, ..
                    }) => {
                        if moving.completed
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
                    PlayerAction::Key(Key {
                        with: ActionKeyWith::Stationary | ActionKeyWith::DoubleJump,
                        ..
                    })
                    | PlayerAction::Move(_)
                    | PlayerAction::SolveRune => None,
                    _ => unreachable!(),
                },
                || Player::UpJumping(up_jumping.moving(moving)),
            )
        }
    }
}

fn perform_mage_up_jump(
    context: &Context,
    state: &PlayerState,
    moving: &mut Moving,
    up_jumping: &mut UpJumping,
    y_distance: i32,
) {
    let jump_key = state.config.jump_key;
    let teleport_key = state.config.teleport_key.expect("has teleport key");

    if y_distance < TELEPORT_WITH_JUMP_THRESHOLD {
        let _ = context.input.send_key(teleport_key);
        *moving = moving.completed(true);
        return;
    }

    if !up_jumping.mage_use_teleport_after_up_jump || up_jumping.mage_did_up_jump {
        return;
    }
    if state.velocity.1 <= UP_JUMPED_Y_VELOCITY_THRESHOLD {
        if moving.timeout.total >= up_jumping.spam_delay {
            let _ = context.input.send_key(jump_key);
        }
    } else {
        up_jumping.mage_did_up_jump = true;
    }
}

#[inline]
fn up_jumping_kind(up_jump_key: Option<KeyKind>, has_teleport_key: bool) -> UpJumpingKind {
    match (up_jump_key, has_teleport_key) {
        (Some(_), true) | (None, true) => UpJumpingKind::Mage,
        (Some(KeyKind::Up), false) => UpJumpingKind::UpArrow,
        (None, false) => UpJumpingKind::JumpKey,
        (Some(_), false) => UpJumpingKind::SpecificKey,
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use opencv::core::Point;

    use super::{Moving, PlayerState, UpJumping, update_up_jumping_context};
    use crate::{
        bridge::{KeyKind, MockInput},
        context::Context,
        player::{Player, Timeout},
    };

    #[test]
    fn up_jumping_start() {
        let pos = Point::new(5, 5);
        let moving = Moving {
            pos,
            dest: Point::new(5, 20),
            ..Default::default()
        };
        let mut state = PlayerState::default();
        let mut context = Context::new(None, None);
        state.config.jump_key = KeyKind::Space;
        state.last_known_pos = Some(pos);
        state.is_stationary = true;

        let mut keys = MockInput::new();
        keys.expect_send_key_down()
            .withf(|key| matches!(key, KeyKind::Up))
            .returning(|_| Ok(()))
            .once();
        keys.expect_send_key()
            .withf(|key| matches!(key, KeyKind::Space))
            .returning(|_| Ok(()))
            .once();
        context.input = Box::new(keys);
        // Space + Up only
        update_up_jumping_context(&context, &mut state, UpJumping::new(moving));
        let _ = context.input; // drop mock for validation

        state.config.upjump_key = Some(KeyKind::C);
        let mut keys = MockInput::new();
        keys.expect_send_key_down()
            .withf(|key| matches!(key, KeyKind::Up))
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_key()
            .withf(|key| matches!(key, KeyKind::Space))
            .never()
            .returning(|_| Ok(()));
        context.input = Box::new(keys);
        // Up only
        update_up_jumping_context(&context, &mut state, UpJumping::new(moving));
        let _ = context.input; // drop mock for validation

        state.config.teleport_key = Some(KeyKind::Shift);
        let mut keys = MockInput::new();
        keys.expect_send_key_down()
            .withf(|key| matches!(key, KeyKind::Up))
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_key()
            .withf(|key| matches!(key, KeyKind::Space))
            .never()
            .returning(|_| Ok(()));
        context.input = Box::new(keys);
        // Space + Up
        update_up_jumping_context(&context, &mut state, UpJumping::new(moving));
        let _ = context.input; // drop mock for validation
    }

    #[test]
    fn up_jumping_update() {
        let moving_pos = Point::new(7, 1);
        let moving = Moving {
            pos: moving_pos,
            timeout: Timeout {
                started: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut state = PlayerState::default();
        state.last_known_pos = Some(Point::new(7, 7));
        state.velocity = (0.0, 1.36);
        let context = Context::new(None, None);

        // up jumped because y velocity > 1.35
        assert_matches!(
            update_up_jumping_context(&context, &mut state, UpJumping::new(moving)),
            Player::UpJumping(UpJumping {
                moving: Moving {
                    timeout: Timeout {
                        current: 1,
                        total: 1,
                        ..
                    },
                    completed: true,
                    ..
                },
                ..
            })
        );
    }

    #[test]
    fn up_jump_demon_slayer() {
        let pos = Point::new(10, 10);
        let dest = Point::new(10, 30);
        let mut moving = Moving {
            pos,
            dest,
            ..Default::default()
        };
        let mut state = PlayerState::default();
        state.config.upjump_key = Some(KeyKind::Up); // Demon Slayer uses Up
        state.config.jump_key = KeyKind::Space;
        state.last_known_pos = Some(pos);
        state.is_stationary = true;

        let mut keys = MockInput::new();
        keys.expect_send_key_down()
            .withf(|key| *key == KeyKind::Up)
            .never();
        keys.expect_send_key()
            .withf(|key| *key == KeyKind::Space)
            .once()
            .returning(|_| Ok(()));
        let mut context = Context::new(None, None);
        context.input = Box::new(keys);

        // Start by sending Space only
        update_up_jumping_context(&context, &mut state, UpJumping::new(moving));
        let _ = context.input;

        // Update by sending Up
        let mut keys = MockInput::new();
        moving.timeout.total = 7; // SPAM_DELAY
        moving.timeout.started = true;
        keys.expect_send_key()
            .withf(|key| *key == KeyKind::Up)
            .times(2)
            .returning(|_| Ok(()));
        keys.expect_send_key()
            .withf(|key| *key == KeyKind::Space)
            .never();
        context.input = Box::new(keys);
        update_up_jumping_context(&context, &mut state, UpJumping::new(moving));
        update_up_jumping_context(&context, &mut state, UpJumping::new(moving));
        let _ = context.input;
    }

    #[test]
    fn up_jump_mage() {
        let pos = Point::new(10, 10);
        let dest = Point::new(10, 30);
        let mut moving = Moving {
            pos,
            dest,
            ..Default::default()
        };
        let mut state = PlayerState::default();
        state.config.teleport_key = Some(KeyKind::Shift);
        state.config.jump_key = KeyKind::Space;
        state.last_known_pos = Some(pos);
        state.is_stationary = true;

        let mut keys = MockInput::new();
        keys.expect_send_key_down()
            .withf(|key| *key == KeyKind::Up)
            .once()
            .returning(|_| Ok(()));
        keys.expect_send_key()
            .withf(|key| *key == KeyKind::Space)
            .once()
            .returning(|_| Ok(()));
        let mut context = Context::new(None, None);
        context.input = Box::new(keys);

        // Start by sending Up and Space
        update_up_jumping_context(&context, &mut state, UpJumping::new(moving));
        let _ = context.input;

        // Change to started
        moving.timeout.started = true;

        // Not sending any key before delay
        let mut keys = MockInput::new();
        moving.timeout.total = 4; // Before SPAM_DELAY
        keys.expect_send_key().never();
        context.input = Box::new(keys);
        assert_matches!(
            update_up_jumping_context(&context, &mut state, UpJumping::new(moving)),
            Player::UpJumping(UpJumping {
                moving: Moving {
                    completed: false,
                    ..
                },
                ..
            })
        );
        let _ = context.input;

        // Send key after delay
        let mut keys = MockInput::new();
        moving.timeout.total = 7; // At SPAM_DELAY
        keys.expect_send_key()
            .withf(|key| *key == KeyKind::Shift)
            .once()
            .returning(|_| Ok(()));
        context.input = Box::new(keys);
        state.last_known_pos = Some(Point { y: 17, ..pos });
        assert_matches!(
            update_up_jumping_context(&context, &mut state, UpJumping::new(moving)),
            Player::UpJumping(UpJumping {
                moving: Moving {
                    completed: true,
                    ..
                },
                ..
            })
        );
        let _ = context.input;
    }
}
