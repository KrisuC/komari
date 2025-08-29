use std::cmp::Ordering;

use opencv::core::Point;

use super::{
    AutoMob, PingPongDirection, PlayerState, Timeout,
    actions::{Key, PingPong, PlayerAction, on_ping_pong_double_jump_action},
    double_jump::DoubleJumping,
    timeout::{Lifecycle, next_timeout_lifecycle},
};
use crate::{
    ActionKeyDirection, ActionKeyWith, Class, KeyBinding, LinkKeyBinding, Position,
    bridge::KeyKind,
    context::Context,
    player::{
        AUTO_MOB_USE_KEY_X_THRESHOLD, AUTO_MOB_USE_KEY_Y_THRESHOLD, LastMovement, MOVE_TIMEOUT,
        Moving, Player, on_action_state_mut,
    },
};

/// The total number of ticks for changing direction before timing out.
const CHANGE_DIRECTION_TIMEOUT: u32 = 3;

/// The tick to which the actual key will be pressed for [`LinkKeyBinding::Along`].
const LINK_ALONG_PRESS_TICK: u32 = 2;

#[derive(Clone, Copy, Debug)]
enum ActionInfo {
    AutoMobbing { should_terminate: bool },
}

/// The different stages of using key.
#[derive(Clone, Copy, Debug)]
pub enum UseKeyStage {
    /// Checks whether [`ActionKeyWith`] and [`ActionKeyDirection`] are satisfied and stalls
    /// for [`UseKey::wait_before_use_ticks`].
    Precondition,
    /// Changes direction to match [`ActionKeyDirection`].
    ///
    /// Returns to [`UseKeyStage::Precondition`] upon timeout.
    ChangingDirection(Timeout),
    /// Ensures player double jumped or is stationary.
    ///
    /// Returns to [`UseKeyStage::Precondition`] if player is stationary or
    /// transfers to [`Player::DoubleJumping`].
    EnsuringUseWith,
    /// Uses the actual key with optional [`LinkKeyBinding`] and stalls
    /// for [`UseKey::wait_after_use_ticks`].
    Using(Timeout, bool),
    /// Ensures all [`UseKey::count`] times executed.
    Postcondition,
}

#[derive(Clone, Copy, Debug)]
pub struct UseKey {
    key: KeyBinding,
    link_key: Option<LinkKeyBinding>,
    count: u32,
    current_count: u32,
    direction: ActionKeyDirection,
    with: ActionKeyWith,
    wait_before_use_ticks: u32,
    wait_after_use_ticks: u32,
    action_info: Option<ActionInfo>,
    stage: UseKeyStage,
}

impl UseKey {
    #[inline]
    pub fn from_action(action: PlayerAction) -> Self {
        UseKey::from_action_pos(action, None)
    }

    pub fn from_action_pos(action: PlayerAction, pos: Option<Point>) -> Self {
        match action {
            PlayerAction::Key(Key {
                key,
                link_key,
                count,
                direction,
                with,
                wait_before_use_ticks,
                wait_before_use_ticks_random_range,
                wait_after_use_ticks,
                wait_after_use_ticks_random_range,
                ..
            }) => {
                let wait_before =
                    random_wait_ticks(wait_before_use_ticks, wait_before_use_ticks_random_range);
                let wait_after =
                    random_wait_ticks(wait_after_use_ticks, wait_after_use_ticks_random_range);

                Self {
                    key,
                    link_key,
                    count,
                    current_count: 0,
                    direction,
                    with,
                    wait_before_use_ticks: wait_before,
                    wait_after_use_ticks: wait_after,
                    action_info: None,
                    stage: UseKeyStage::Precondition,
                }
            }
            PlayerAction::AutoMob(mob) => {
                let wait_before =
                    random_wait_ticks(mob.wait_before_ticks, mob.wait_before_ticks_random_range);
                let wait_after =
                    random_wait_ticks(mob.wait_after_ticks, mob.wait_after_ticks_random_range);
                let pos = pos.expect("has position");
                let direction = match pos.x.cmp(&mob.position.x) {
                    Ordering::Less => ActionKeyDirection::Right,
                    Ordering::Equal => ActionKeyDirection::Any,
                    Ordering::Greater => ActionKeyDirection::Left,
                };
                let x_distance = (pos.x - mob.position.x).abs();
                let y_distance = (pos.y - mob.position.y).abs();
                let should_terminate = x_distance <= AUTO_MOB_USE_KEY_X_THRESHOLD
                    && y_distance <= AUTO_MOB_USE_KEY_Y_THRESHOLD;

                Self {
                    key: mob.key,
                    link_key: mob.link_key,
                    count: mob.count,
                    current_count: 0,
                    direction,
                    with: mob.with,
                    wait_before_use_ticks: wait_before,
                    wait_after_use_ticks: wait_after,
                    action_info: Some(ActionInfo::AutoMobbing { should_terminate }),
                    stage: UseKeyStage::Precondition,
                }
            }
            PlayerAction::PingPong(ping_pong) => {
                let wait_before = random_wait_ticks(
                    ping_pong.wait_before_ticks,
                    ping_pong.wait_before_ticks_random_range,
                );
                let wait_after = random_wait_ticks(
                    ping_pong.wait_after_ticks,
                    ping_pong.wait_after_ticks_random_range,
                );
                let direction = if matches!(ping_pong.direction, PingPongDirection::Left) {
                    ActionKeyDirection::Left
                } else {
                    ActionKeyDirection::Right
                };

                Self {
                    key: ping_pong.key,
                    link_key: ping_pong.link_key,
                    count: ping_pong.count,
                    current_count: 0,
                    direction,
                    with: ping_pong.with,
                    wait_before_use_ticks: wait_before,
                    wait_after_use_ticks: wait_after,
                    action_info: None,
                    stage: UseKeyStage::Precondition,
                }
            }
            _ => unreachable!(),
        }
    }
}

/// Updates the [`Player::UseKey`] contextual state.
///
/// Like [`Player::SolvingRune`], this state can only be transitioned via a [`PlayerAction`]. It
/// can be transitioned during any of the movement state. Or if there is no position, it will
/// be transitioned to immediately by [`Player::Idle`].
///
/// There are multiple stages to using a key as described by [`UseKeyStage`].
pub fn update_use_key_context(
    context: &Context,
    state: &mut PlayerState,
    use_key: UseKey,
) -> Player {
    // TODO: Am I cooked?
    let next = match use_key.stage {
        UseKeyStage::Precondition => {
            debug_assert!(use_key.current_count < use_key.count);
            if !ensure_direction(state, use_key.direction) {
                return Player::UseKey(UseKey {
                    stage: UseKeyStage::ChangingDirection(Timeout::default()),
                    ..use_key
                });
            }
            if !ensure_use_with(state, use_key) {
                return Player::UseKey(UseKey {
                    stage: UseKeyStage::EnsuringUseWith,
                    ..use_key
                });
            }
            debug_assert!(
                matches!(use_key.direction, ActionKeyDirection::Any)
                    || use_key.direction == state.last_known_direction
            );
            debug_assert!(
                matches!(use_key.with, ActionKeyWith::Any)
                    || (matches!(use_key.with, ActionKeyWith::Stationary) && state.is_stationary)
                    || (matches!(use_key.with, ActionKeyWith::DoubleJump)
                        && matches!(state.last_movement, Some(LastMovement::DoubleJumping)))
            );
            let next = Player::UseKey(UseKey {
                stage: UseKeyStage::Using(Timeout::default(), false),
                ..use_key
            });
            if use_key.wait_before_use_ticks > 0 {
                state.stalling_timeout_state = Some(next);
                Player::Stalling(Timeout::default(), use_key.wait_before_use_ticks)
            } else {
                state.use_immediate_control_flow = true;
                next
            }
        }
        UseKeyStage::ChangingDirection(timeout) => {
            let key = match use_key.direction {
                ActionKeyDirection::Left => KeyKind::Left,
                ActionKeyDirection::Right => KeyKind::Right,
                ActionKeyDirection::Any => unreachable!(),
            };
            match next_timeout_lifecycle(timeout, CHANGE_DIRECTION_TIMEOUT) {
                Lifecycle::Started(timeout) => {
                    let _ = context.input.send_key_down(key);
                    Player::UseKey(UseKey {
                        stage: UseKeyStage::ChangingDirection(timeout),
                        ..use_key
                    })
                }
                Lifecycle::Ended => {
                    let _ = context.input.send_key_up(key);
                    state.last_known_direction = use_key.direction;
                    Player::UseKey(UseKey {
                        stage: UseKeyStage::Precondition,
                        ..use_key
                    })
                }
                Lifecycle::Updated(timeout) => Player::UseKey(UseKey {
                    stage: UseKeyStage::ChangingDirection(timeout),
                    ..use_key
                }),
            }
        }
        UseKeyStage::EnsuringUseWith => match use_key.with {
            ActionKeyWith::Any => unreachable!(),
            ActionKeyWith::Stationary => {
                let stage = if state.is_stationary {
                    UseKeyStage::Precondition
                } else {
                    UseKeyStage::EnsuringUseWith
                };
                Player::UseKey(UseKey { stage, ..use_key })
            }
            ActionKeyWith::DoubleJump => {
                let pos = state.last_known_pos.expect("in positional context");
                Player::DoubleJumping(DoubleJumping::new(
                    Moving::new(pos, pos, false, None),
                    true,
                    true,
                ))
            }
        },
        UseKeyStage::Using(timeout, completed) => {
            debug_assert!(use_key.link_key.is_some() || !completed);
            debug_assert!(state.stalling_timeout_state.is_none());
            match use_key.link_key {
                Some(LinkKeyBinding::After(_)) => {
                    if !timeout.started {
                        let _ = context.input.send_key(use_key.key.into());
                    }
                    if !completed {
                        return update_link_key(
                            context,
                            state.config.class,
                            state.config.jump_key,
                            use_key,
                            timeout,
                            completed,
                        );
                    }
                }
                Some(LinkKeyBinding::AtTheSame(key)) => {
                    let _ = context.input.send_key(key.into());
                    let _ = context.input.send_key(use_key.key.into());
                }
                Some(LinkKeyBinding::Along(_)) => {
                    if !completed {
                        return update_link_key(
                            context,
                            state.config.class,
                            state.config.jump_key,
                            use_key,
                            timeout,
                            completed,
                        );
                    }
                }
                Some(LinkKeyBinding::Before(_)) | None => {
                    if use_key.link_key.is_some() && !completed {
                        return update_link_key(
                            context,
                            state.config.class,
                            state.config.jump_key,
                            use_key,
                            timeout,
                            completed,
                        );
                    }
                    debug_assert!(use_key.link_key.is_none() || completed);
                    let _ = context.input.send_key(use_key.key.into());
                }
            }
            let next = Player::UseKey(UseKey {
                stage: UseKeyStage::Postcondition,
                ..use_key
            });
            if use_key.wait_after_use_ticks > 0 {
                state.stalling_timeout_state = Some(next);
                Player::Stalling(Timeout::default(), use_key.wait_after_use_ticks)
            } else {
                next
            }
        }
        UseKeyStage::Postcondition => {
            debug_assert!(state.stalling_timeout_state.is_none());
            if use_key.current_count + 1 < use_key.count {
                Player::UseKey(UseKey {
                    current_count: use_key.current_count + 1,
                    stage: UseKeyStage::Precondition,
                    ..use_key
                })
            } else {
                Player::Idle
            }
        }
    };

    on_action_state_mut(
        state,
        |state, action| match action {
            PlayerAction::AutoMob(AutoMob {
                position: Position { y, .. },
                ..
            }) => {
                let should_terminate = matches!(
                    use_key.action_info,
                    Some(ActionInfo::AutoMobbing {
                        should_terminate: true
                    })
                );
                let is_terminal = should_terminate && matches!(next, Player::Idle);
                if is_terminal {
                    state.auto_mob_track_ignore_xs(context, false);
                    if state.auto_mob_reachable_y_require_update(y) {
                        return Some((Player::Stalling(Timeout::default(), MOVE_TIMEOUT), false));
                    }
                }
                Some((next, is_terminal))
            }
            PlayerAction::PingPong(PingPong {
                bound, direction, ..
            }) => {
                if matches!(next, Player::Idle) {
                    state.clear_unstucking(true);
                    Some(on_ping_pong_double_jump_action(
                        context,
                        state.last_known_pos.expect("in positional context"),
                        bound,
                        direction,
                    ))
                } else {
                    None
                }
            }
            PlayerAction::Key(_) => Some((next, matches!(next, Player::Idle))),
            PlayerAction::Move(_) => None,
            _ => unreachable!(),
        },
        || next,
    )
}

#[inline]
fn ensure_direction(state: &PlayerState, direction: ActionKeyDirection) -> bool {
    match direction {
        ActionKeyDirection::Any => true,
        ActionKeyDirection::Left | ActionKeyDirection::Right => {
            direction == state.last_known_direction
        }
    }
}

#[inline]
fn ensure_use_with(state: &PlayerState, use_key: UseKey) -> bool {
    match use_key.with {
        ActionKeyWith::Any => true,
        ActionKeyWith::Stationary => state.is_stationary,
        ActionKeyWith::DoubleJump => {
            matches!(state.last_movement, Some(LastMovement::DoubleJumping))
        }
    }
}

#[inline]
fn update_link_key(
    context: &Context,
    class: Class,
    jump_key: KeyKind,
    use_key: UseKey,
    timeout: Timeout,
    completed: bool,
) -> Player {
    debug_assert!(!timeout.started || !completed);
    let link_key = use_key.link_key.unwrap();
    let link_key_timeout = if matches!(link_key, LinkKeyBinding::Along(_)) {
        4
    } else {
        match class {
            Class::Cadena => 4,
            Class::Blaster => 8,
            Class::Ark => 10,
            Class::Generic => 5,
        }
    };

    match next_timeout_lifecycle(timeout, link_key_timeout) {
        Lifecycle::Started(timeout) => {
            if let LinkKeyBinding::Before(key) = link_key {
                let _ = context.input.send_key(key.into());
            } else if let LinkKeyBinding::Along(key) = link_key {
                let _ = context.input.send_key_down(key.into());
            }
            Player::UseKey(UseKey {
                stage: UseKeyStage::Using(timeout, completed),
                ..use_key
            })
        }
        Lifecycle::Ended => {
            if let LinkKeyBinding::After(key) = link_key {
                let _ = context.input.send_key(key.into());
                if matches!(class, Class::Blaster) && KeyKind::from(key) != jump_key {
                    let _ = context.input.send_key(jump_key);
                }
            } else if let LinkKeyBinding::Along(key) = link_key {
                let _ = context.input.send_key_up(key.into());
            }
            Player::UseKey(UseKey {
                stage: UseKeyStage::Using(timeout, true),
                ..use_key
            })
        }
        Lifecycle::Updated(timeout) => {
            if matches!(link_key, LinkKeyBinding::Along(_))
                && timeout.total == LINK_ALONG_PRESS_TICK
            {
                let _ = context.input.send_key(use_key.key.into());
            }
            Player::UseKey(UseKey {
                stage: UseKeyStage::Using(timeout, completed),
                ..use_key
            })
        }
    }
}

#[inline]
fn random_wait_ticks(wait_base_ticks: u32, wait_random_range: u32) -> u32 {
    // TODO: Replace rand with Rng
    let wait_min = wait_base_ticks.saturating_sub(wait_random_range);
    let wait_max = wait_base_ticks.saturating_add(wait_random_range + 1);
    rand::random_range(wait_min..wait_max)
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use crate::{
        ActionKeyDirection, ActionKeyWith, KeyBinding, LinkKeyBinding,
        bridge::{KeyKind, MockInput},
        context::Context,
        player::{
            Player, PlayerState, Timeout, update_non_positional_context,
            use_key::{UseKey, UseKeyStage, update_use_key_context},
        },
    };

    #[test]
    fn use_key_ensure_use_with() {
        let mut state = PlayerState::default();
        let context = Context::new(None, None);
        let use_key = UseKey {
            key: KeyBinding::A,
            link_key: None,
            count: 1,
            current_count: 0,
            direction: ActionKeyDirection::Any,
            with: ActionKeyWith::Stationary,
            wait_before_use_ticks: 0,
            wait_after_use_ticks: 0,
            action_info: None,
            stage: UseKeyStage::Precondition,
        };

        // ensuring use with start
        let mut player = Player::UseKey(use_key);
        player = update_non_positional_context(player, &context, &mut state, false).unwrap();
        assert_matches!(
            player,
            Player::UseKey(UseKey {
                stage: UseKeyStage::EnsuringUseWith,
                ..
            })
        );

        // ensuring use with complete
        state.is_stationary = true;
        player = update_non_positional_context(player, &context, &mut state, false).unwrap();
        assert_matches!(
            player,
            Player::UseKey(UseKey {
                stage: UseKeyStage::Precondition,
                ..
            })
        );
    }

    #[test]
    fn use_key_change_direction() {
        let mut keys = MockInput::new();
        keys.expect_send_key_down()
            .withf(|key| matches!(key, KeyKind::Left))
            .returning(|_| Ok(()));
        keys.expect_send_key_up()
            .withf(|key| matches!(key, KeyKind::Left))
            .returning(|_| Ok(()));
        let mut state = PlayerState::default();
        let context = Context::new(Some(keys), None);
        let use_key = UseKey {
            key: KeyBinding::A,
            link_key: None,
            count: 1,
            current_count: 0,
            direction: ActionKeyDirection::Left,
            with: ActionKeyWith::Any,
            wait_before_use_ticks: 0,
            wait_after_use_ticks: 0,
            action_info: None,
            stage: UseKeyStage::Precondition,
        };

        // changing direction
        let mut player = Player::UseKey(use_key);
        player = update_non_positional_context(player, &context, &mut state, false).unwrap();
        assert_matches!(state.last_known_direction, ActionKeyDirection::Any);
        assert_matches!(
            player,
            Player::UseKey(UseKey {
                stage: UseKeyStage::ChangingDirection(Timeout { started: false, .. }),
                ..
            })
        );

        // changing direction start
        player = update_non_positional_context(player, &context, &mut state, false).unwrap();
        assert_matches!(state.last_known_direction, ActionKeyDirection::Any);
        assert_matches!(
            player,
            Player::UseKey(UseKey {
                stage: UseKeyStage::ChangingDirection(Timeout { started: true, .. }),
                ..
            })
        );

        // changing direction complete
        let mut player = Player::UseKey(UseKey {
            stage: UseKeyStage::ChangingDirection(Timeout {
                started: true,
                current: 3,
                total: 3,
            }),
            ..use_key
        });
        player = update_non_positional_context(player, &context, &mut state, false).unwrap();
        assert_matches!(state.last_known_direction, ActionKeyDirection::Left);
        assert_matches!(
            player,
            Player::UseKey(UseKey {
                stage: UseKeyStage::Precondition,
                ..
            })
        )
    }

    #[test]
    fn use_key_count() {
        let mut keys = MockInput::new();
        keys.expect_send_key()
            .times(100)
            .withf(|key| matches!(key, KeyKind::A))
            .returning(|_| Ok(()));
        let mut state = PlayerState::default();
        let context = Context::new(Some(keys), None);
        let use_key = UseKey {
            key: KeyBinding::A,
            link_key: None,
            count: 100,
            current_count: 0,
            direction: ActionKeyDirection::Any,
            with: ActionKeyWith::Any,
            wait_before_use_ticks: 0,
            wait_after_use_ticks: 0,
            action_info: None,
            stage: UseKeyStage::Precondition,
        };

        let mut player = Player::UseKey(use_key);
        for i in 0..100 {
            player = update_non_positional_context(player, &context, &mut state, false).unwrap();
            assert_matches!(
                player,
                Player::UseKey(UseKey {
                    stage: UseKeyStage::Using(_, _),
                    ..
                })
            );
            player = update_non_positional_context(player, &context, &mut state, false).unwrap();
            assert_matches!(
                player,
                Player::UseKey(UseKey {
                    stage: UseKeyStage::Postcondition,
                    ..
                })
            );
            player = update_non_positional_context(player, &context, &mut state, false).unwrap();
            if i == 99 {
                assert_matches!(player, Player::Idle);
            } else {
                assert_matches!(
                    player,
                    Player::UseKey(UseKey {
                        stage: UseKeyStage::Precondition,
                        ..
                    })
                );
            }
        }
    }

    #[test]
    fn use_key_stalling() {
        let mut keys = MockInput::new();
        keys.expect_send_key()
            .withf(|key| matches!(key, KeyKind::A))
            .return_once(|_| Ok(()));
        let mut state = PlayerState::default();
        let context = Context::new(Some(keys), None);
        let use_key = UseKey {
            key: KeyBinding::A,
            link_key: None,
            count: 1,
            current_count: 0,
            direction: ActionKeyDirection::Any,
            with: ActionKeyWith::Any,
            wait_before_use_ticks: 10,
            wait_after_use_ticks: 20,
            action_info: None,
            stage: UseKeyStage::Precondition,
        };

        // enter stalling state
        assert!(state.stalling_timeout_state.is_none());
        assert_matches!(
            update_use_key_context(&context, &mut state, use_key),
            Player::Stalling(_, 10)
        );
        assert_matches!(
            state.stalling_timeout_state,
            Some(Player::UseKey(UseKey {
                stage: UseKeyStage::Using(_, false),
                ..
            }))
        );

        // complete before stalling state and send key
        assert_matches!(
            update_non_positional_context(
                state.stalling_timeout_state.take().unwrap(),
                &context,
                &mut state,
                false
            ),
            Some(Player::Stalling(_, 20))
        );
        assert_matches!(
            state.stalling_timeout_state,
            Some(Player::UseKey(UseKey {
                stage: UseKeyStage::Postcondition,
                ..
            }))
        );

        // complete after stalling state and return idle
        assert_matches!(
            update_non_positional_context(
                state.stalling_timeout_state.take().unwrap(),
                &context,
                &mut state,
                false
            ),
            Some(Player::Idle)
        );
    }

    #[test]
    fn use_key_link_along() {
        let mut state = PlayerState::default();
        let mut context = Context::new(None, None);
        let mut use_key = UseKey {
            key: KeyBinding::A,
            link_key: Some(LinkKeyBinding::Along(KeyBinding::Alt)),
            count: 1,
            current_count: 0,
            direction: ActionKeyDirection::Any,
            with: ActionKeyWith::Any,
            wait_before_use_ticks: 0,
            wait_after_use_ticks: 0,
            action_info: None,
            stage: UseKeyStage::Using(Timeout::default(), false),
        };

        // Starts by holding down Alt key
        let mut keys = MockInput::new();
        keys.expect_send_key_down()
            .withf(|key| matches!(key, KeyKind::Alt))
            .once()
            .return_once(|_| Ok(()));
        context.input = Box::new(keys);
        update_use_key_context(&context, &mut state, use_key);
        let _ = context.input; // test check point by dropping

        // Sends A at tick 2
        let mut keys = MockInput::new();
        keys.expect_send_key()
            .withf(|key| matches!(key, KeyKind::A))
            .once()
            .return_once(|_| Ok(()));
        context.input = Box::new(keys);
        use_key.stage = UseKeyStage::Using(
            Timeout {
                started: true,
                total: 1,
                current: 1,
            },
            false,
        );
        assert_matches!(
            update_use_key_context(&context, &mut state, use_key),
            Player::UseKey(UseKey {
                stage: UseKeyStage::Using(
                    Timeout {
                        total: 2,
                        current: 2,
                        ..
                    },
                    false
                ),
                ..
            })
        );
        let _ = context.input; // test check point by dropping

        // Ends by releasing Alt
        let mut keys = MockInput::new();
        keys.expect_send_key_up()
            .withf(|key| matches!(key, KeyKind::Alt))
            .once()
            .return_once(|_| Ok(()));
        context.input = Box::new(keys);
        use_key.stage = UseKeyStage::Using(
            Timeout {
                started: true,
                total: 4,
                current: 4,
            },
            false,
        );
        assert_matches!(
            update_use_key_context(&context, &mut state, use_key),
            Player::UseKey(UseKey {
                stage: UseKeyStage::Using(
                    Timeout {
                        total: 4,
                        current: 4,
                        ..
                    },
                    true
                ),
                ..
            })
        );
        // test check point by dropping here
    }
}
