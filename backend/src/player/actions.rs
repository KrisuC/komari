use std::fmt;

use opencv::core::{Point, Rect};
use strum::Display;

use super::{Player, PlayerState, use_key::UseKey};
use crate::{
    Action, ActionKey, ActionKeyDirection, ActionKeyWith, ActionMove, FamiliarRarity, KeyBinding,
    Position, SwappableFamiliars,
    array::Array,
    bridge::KeyKind,
    context::{Context, MS_PER_TICK},
    database::LinkKeyBinding,
    minimap::Minimap,
};

/// The minimum x distance required to transition to [`Player::UseKey`] in auto mob action.
const AUTO_MOB_USE_KEY_X_THRESHOLD: i32 = 16;

/// The minimum y distance required to transition to [`Player::UseKey`] in auto mob action.
const AUTO_MOB_USE_KEY_Y_THRESHOLD: i32 = 8;

/// Represents the fixed key action.
///
/// Converted from [`ActionKey`] without fields used by [`Rotator`]
#[derive(Clone, Copy, Debug)]
pub struct Key {
    pub key: KeyBinding,
    pub link_key: Option<LinkKeyBinding>,
    pub count: u32,
    pub position: Option<Position>,
    pub direction: ActionKeyDirection,
    pub with: ActionKeyWith,
    pub wait_before_use_ticks: u32,
    pub wait_before_use_ticks_random_range: u32,
    pub wait_after_use_ticks: u32,
    pub wait_after_use_ticks_random_range: u32,
}

impl From<ActionKey> for Key {
    fn from(
        ActionKey {
            key,
            link_key,
            count,
            position,
            direction,
            with,
            wait_before_use_millis,
            wait_before_use_millis_random_range,
            wait_after_use_millis,
            wait_after_use_millis_random_range,
            ..
        }: ActionKey,
    ) -> Self {
        let count = count.max(1);
        let wait_before_use_ticks = (wait_before_use_millis / MS_PER_TICK) as u32;
        let wait_before_use_ticks_random_range =
            (wait_before_use_millis_random_range / MS_PER_TICK) as u32;
        let wait_after_use_ticks = (wait_after_use_millis / MS_PER_TICK) as u32;
        let wait_after_use_ticks_random_range =
            (wait_after_use_millis_random_range / MS_PER_TICK) as u32;

        Self {
            key,
            link_key,
            count,
            position,
            direction,
            with,
            wait_before_use_ticks,
            wait_before_use_ticks_random_range,
            wait_after_use_ticks,
            wait_after_use_ticks_random_range,
        }
    }
}

/// Represents the fixed move action.
///
/// Converted from [`ActionMove`] without fields used by [`Rotator`].
#[derive(Clone, Copy, Debug)]
pub struct Move {
    pub position: Position,
    pub wait_after_move_ticks: u32,
}

impl From<ActionMove> for Move {
    fn from(
        ActionMove {
            position,
            wait_after_move_millis,
            ..
        }: ActionMove,
    ) -> Self {
        Self {
            position,
            wait_after_move_ticks: (wait_after_move_millis / MS_PER_TICK) as u32,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct AutoMob {
    pub key: KeyBinding,
    pub link_key: Option<LinkKeyBinding>,
    pub count: u32,
    pub with: ActionKeyWith,
    pub wait_before_ticks: u32,
    pub wait_before_ticks_random_range: u32,
    pub wait_after_ticks: u32,
    pub wait_after_ticks_random_range: u32,
    pub position: Position,
}

impl fmt::Display for AutoMob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}", self.position.x, self.position.y)
    }
}

/// Represents a ping pong action.
///
/// This is a type of action that moves in one direction and spams a fixed key. Once the player hits
/// either edges determined by [`Self::bound`] or close enough, the action is completed.
/// The [`Rotator`] then rotates the next action in the reverse direction.
///
/// This action forces the player to always stay inside the bound.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct PingPong {
    pub key: KeyBinding,
    pub link_key: Option<LinkKeyBinding>,
    pub count: u32,
    pub with: ActionKeyWith,
    pub wait_before_ticks: u32,
    pub wait_before_ticks_random_range: u32,
    pub wait_after_ticks: u32,
    pub wait_after_ticks_random_range: u32,
    /// Bound of ping pong action.
    ///
    /// This bound is in player relative coordinate.
    pub bound: Rect,
    pub direction: PingPongDirection,
}

#[derive(Clone, Copy, Debug)]
pub enum PingPongDirection {
    Left,
    Right,
}

#[cfg(test)]
impl Default for PingPongDirection {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FamiliarsSwap {
    pub swappable_slots: SwappableFamiliars,
    pub swappable_rarities: Array<FamiliarRarity, 2>,
}

#[derive(Clone, Copy, Debug)]
pub struct Panic {
    pub to: PanicTo,
}

#[derive(Clone, Copy, Debug)]
pub enum PanicTo {
    Town,
    Channel,
}

#[derive(Clone, Debug)]
pub struct Chat {
    pub content: String,
}

/// Represents an action the [`Rotator`] can use.
#[derive(Clone, Debug, Display)]
pub enum PlayerAction {
    /// Fixed key action provided by the user.
    Key(Key),
    /// Fixed move action provided by the user.
    Move(Move),
    /// Solves rune action.
    SolveRune,
    /// Auto-mobbing action.
    #[strum(to_string = "AutoMob({0})")]
    AutoMob(AutoMob),
    /// Ping pong action.
    PingPong(PingPong),
    /// Swaps familiars action.
    FamiliarsSwap(FamiliarsSwap),
    /// Panics to town or another channel action.
    Panic(Panic),
    /// Chats in-game action.
    Chat(Chat),
}

impl From<Action> for PlayerAction {
    fn from(action: Action) -> Self {
        match action {
            Action::Move(action) => PlayerAction::Move(action.into()),
            Action::Key(action) => PlayerAction::Key(action.into()),
        }
    }
}

#[inline]
pub(super) fn on_ping_pong_double_jump_action(
    context: &Context,
    cur_pos: Point,
    bound: Rect,
    direction: PingPongDirection,
) -> (Player, bool) {
    let hit_x_bound_edge = match direction {
        PingPongDirection::Left => cur_pos.x - bound.x <= 0,
        PingPongDirection::Right => cur_pos.x - bound.x - bound.width >= 0,
    };
    if hit_x_bound_edge {
        return (Player::Idle, true);
    }

    let _ = context.input.send_key_up(KeyKind::Down);
    let _ = context.input.send_key_up(KeyKind::Up);
    let _ = context.input.send_key_up(KeyKind::Left);
    let _ = context.input.send_key_up(KeyKind::Right);
    let minimap_width = match context.minimap {
        Minimap::Idle(idle) => idle.bbox.width,
        _ => unreachable!(),
    };
    let y = cur_pos.y; // y doesn't matter in ping pong
    let moving = match direction {
        PingPongDirection::Left => Player::Moving(Point::new(0, y), false, None),
        PingPongDirection::Right => Player::Moving(Point::new(minimap_width, y), false, None),
    };
    (moving, false)
}

/// Checks proximity in [`PlayerAction::AutoMob`] for transitioning to [`Player::UseKey`].
///
/// This is common logics shared with other contextual states when there is auto mob action.
#[inline]
pub(super) fn on_auto_mob_use_key_action(
    context: &Context,
    action: PlayerAction,
    cur_pos: Point,
    x_distance: i32,
    y_distance: i32,
) -> Option<(Player, bool)> {
    if x_distance <= AUTO_MOB_USE_KEY_X_THRESHOLD && y_distance <= AUTO_MOB_USE_KEY_Y_THRESHOLD {
        let _ = context.input.send_key_up(KeyKind::Down);
        let _ = context.input.send_key_up(KeyKind::Up);
        let _ = context.input.send_key_up(KeyKind::Left);
        let _ = context.input.send_key_up(KeyKind::Right);
        Some((
            Player::UseKey(UseKey::from_action_pos(action, Some(cur_pos))),
            false,
        ))
    } else {
        None
    }
}

/// Callbacks for when there is a normal or priority [`PlayerAction`].
///
/// This version does not require [`PlayerState`] in the callbacks arguments.
#[inline]
pub(super) fn on_action(
    state: &mut PlayerState,
    on_action_context: impl FnOnce(PlayerAction) -> Option<(Player, bool)>,
    on_default_context: impl FnOnce() -> Player,
) -> Player {
    on_action_state_mut(
        state,
        |_, action| on_action_context(action),
        on_default_context,
    )
}

/// Callbacks for when there is a normal or priority [`PlayerAction`].
///
/// This version requires a shared reference [`PlayerState`] in the callbacks arguments.
#[inline]
pub(super) fn on_action_state(
    state: &mut PlayerState,
    on_action_context: impl FnOnce(&PlayerState, PlayerAction) -> Option<(Player, bool)>,
    on_default_context: impl FnOnce() -> Player,
) -> Player {
    on_action_state_mut(
        state,
        |state, action| on_action_context(state, action),
        on_default_context,
    )
}

/// Callbacks for when there is a normal or priority [`PlayerAction`].
///
/// When there is a priority action, it takes precendece over the normal action. The callback
/// should return a tuple [`Option<(Player, bool)>`] with:
/// - `Some((Player, false))` indicating the callback is handled but `Player` is not terminal state
/// - `Some((Player, true))` indicating the callback is handled and `Player` is terminal state
/// - `None` indicating the callback is not handled and will be defaulted to `on_default_context`
///
/// When the returned tuple indicates a terminal state, the `PlayerAction` is considered complete.
/// Because this function passes a mutable reference of `PlayerState` to `on_action_context`,
/// caller should be aware not to clear the action but let this function handles it.
#[inline]
pub(super) fn on_action_state_mut(
    state: &mut PlayerState,
    on_action_context: impl FnOnce(&mut PlayerState, PlayerAction) -> Option<(Player, bool)>,
    on_default_context: impl FnOnce() -> Player,
) -> Player {
    if let Some(action) = state
        .priority_action
        .clone()
        .or(state.normal_action.clone())
        && let Some((next, is_terminal)) = on_action_context(state, action.clone())
    {
        debug_assert!(state.has_normal_action() || state.has_priority_action());
        if is_terminal {
            match action {
                PlayerAction::SolveRune
                | PlayerAction::PingPong(_)
                | PlayerAction::Move(_)
                | PlayerAction::Key(Key {
                    position: Some(Position { .. }),
                    ..
                }) => {
                    state.clear_unstucking(false);
                }
                _ => (),
            }
            // FIXME: clear only when has position?
            state.clear_action_completed();
        }
        return next;
    }
    on_default_context()
}
