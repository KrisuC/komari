use std::ops::Range;

use log::{debug, info};
use opencv::core::Point;

use super::{
    GRAPPLING_MAX_THRESHOLD, JUMP_THRESHOLD, Player, PlayerState,
    actions::{Key, Move, PlayerAction},
    double_jump::{DOUBLE_JUMP_THRESHOLD, DoubleJumping},
    state::LastMovement,
    timeout::Timeout,
    up_jump::UpJumping,
};
use crate::{
    ActionKeyDirection, ActionKeyWith, MAX_PLATFORMS_COUNT,
    array::Array,
    bridge::KeyKind,
    context::Context,
    pathing::{MovementHint, PlatformWithNeighbors, find_points_with},
    player::{
        adjust::{ADJUSTING_MEDIUM_THRESHOLD, ADJUSTING_SHORT_THRESHOLD, Adjusting},
        grapple::GRAPPLING_THRESHOLD,
        on_action,
        solve_rune::SolvingRune,
        use_key::UseKey,
    },
};

/// Maximum amount of ticks a change in x or y direction must be detected.
pub const MOVE_TIMEOUT: u32 = 5;

/// Jumpable y distances.
const JUMPABLE_RANGE: Range<i32> = 4..JUMP_THRESHOLD;
const UP_JUMP_THRESHOLD: i32 = 10;

/// Intermediate points to move by.
///
/// The last point is the destination.
#[derive(Clone, Copy, Debug)]
pub struct MovingIntermediates {
    current: usize,
    inner: Array<(Point, MovementHint, bool), 16>,
}

impl MovingIntermediates {
    #[inline]
    pub fn inner(&self) -> Array<(Point, MovementHint, bool), 16> {
        self.inner
    }

    #[inline]
    pub fn has_next(&self) -> bool {
        self.current < self.inner.len()
    }

    #[inline]
    pub fn next(&mut self) -> Option<(Point, bool)> {
        if self.current >= self.inner.len() {
            return None;
        }
        let next = self.inner[self.current];
        self.current += 1;
        Some((next.0, next.2))
    }
}

/// A contextual state that stores moving-related data.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct Moving {
    /// The player's previous position.
    ///
    /// It will be updated to current position after calling [`next_moving_lifecycle_with_axis`].
    /// Before calling this function, it will always be the previous position in relative to
    /// [`PlayerState::last_known_pos`].
    pub pos: Point,
    /// The destination the player is moving to.
    ///
    /// When [`Self::intermediates`] is [`Some`], this could be an intermediate destination.
    pub dest: Point,
    /// Whether to allow adjusting to precise destination.
    pub exact: bool,
    /// Whether the movement has completed.
    ///
    /// For example, in up jump with fixed key like Corsair, it is considered complete
    /// when the key is pressed.
    pub completed: bool,
    /// Current timeout ticks for checking if the player position's changed.
    pub timeout: Timeout,
    /// Intermediate points to move to before reaching the destination.
    ///
    /// When [`Some`], the last point is the destination.
    pub intermediates: Option<MovingIntermediates>,
}

/// Convenient implementations
impl Moving {
    #[inline]
    pub fn new(
        pos: Point,
        dest: Point,
        exact: bool,
        intermediates: Option<MovingIntermediates>,
    ) -> Self {
        Self {
            pos,
            dest,
            exact,
            completed: false,
            timeout: Timeout::default(),
            intermediates,
        }
    }

    #[inline]
    pub fn completed(self, completed: bool) -> Moving {
        Moving { completed, ..self }
    }

    #[inline]
    pub fn timeout(self, timeout: Timeout) -> Moving {
        Moving { timeout, ..self }
    }

    #[inline]
    pub fn timeout_current(self, current: u32) -> Moving {
        Moving {
            timeout: Timeout {
                current,
                ..self.timeout
            },
            ..self
        }
    }

    #[inline]
    pub fn timeout_started(self, started: bool) -> Moving {
        Moving {
            timeout: Timeout {
                started,
                ..self.timeout
            },
            ..self
        }
    }

    #[inline]
    fn intermediate_hint(&self) -> Option<MovementHint> {
        self.intermediates
            .map(|intermediates| intermediates.inner[intermediates.current.saturating_sub(1)].1)
    }

    /// Computes the x distance and direction between [`Self::dest`] and `cur_pos`.
    ///
    /// If `current_destination` is false, it will use the last destination if
    /// [`Self::intermediates`] is [`Some`].
    ///
    /// Returns the distance and direction values pair computed from `dest - cur_pos`.
    #[inline]
    pub fn x_distance_direction_from(
        &self,
        current_destination: bool,
        cur_pos: Point,
    ) -> (i32, i32) {
        self.distance_direction_from(true, current_destination, cur_pos)
    }

    /// Computes the y distance and direction between [`Self::dest`] and `cur_pos`.
    ///
    /// If `current_destination` is false, it will use the last destination if
    /// [`Self::intermediates`] is [`Some`].
    ///
    /// Returns the distance and direction values pair computed from `dest - cur_pos`.
    #[inline]
    pub fn y_distance_direction_from(
        &self,
        current_destination: bool,
        cur_pos: Point,
    ) -> (i32, i32) {
        self.distance_direction_from(false, current_destination, cur_pos)
    }

    #[inline]
    fn distance_direction_from(
        &self,
        compute_x: bool,
        current_destination: bool,
        cur_pos: Point,
    ) -> (i32, i32) {
        let dest = if current_destination {
            self.dest
        } else {
            self.last_destination()
        };
        let direction = if compute_x {
            dest.x - cur_pos.x
        } else {
            dest.y - cur_pos.y
        };
        let distance = direction.abs();
        (distance, direction)
    }

    #[inline]
    fn last_destination(&self) -> Point {
        if self.is_destination_intermediate() {
            let points = self.intermediates.unwrap().inner;
            points[points.len() - 1].0
        } else {
            self.dest
        }
    }

    #[inline]
    pub fn is_destination_intermediate(&self) -> bool {
        self.intermediates
            .is_some_and(|intermediates| intermediates.has_next())
    }

    /// Determines whether auto mobbing intermediate destination can be skipped.
    #[inline]
    pub fn auto_mob_can_skip_current_destination(&self, state: &PlayerState) -> bool {
        if !state.has_auto_mob_action_only() {
            return false;
        }

        let Some(intermediates) = self.intermediates else {
            return false;
        };
        if !intermediates.has_next() {
            return false;
        }

        let pos = state.last_known_pos.expect("in positional context");
        let (x_distance, _) = self.x_distance_direction_from(true, pos);
        let (y_distance, y_direction) = self.y_distance_direction_from(true, pos);

        let did_fall_down =
            matches!(state.last_movement, Some(LastMovement::Falling)) && y_direction >= 0;
        let did_up_jump =
            matches!(state.last_movement, Some(LastMovement::UpJumping)) && y_direction <= 0;
        let y_within_jump = y_distance < JUMP_THRESHOLD;

        let can_skip_y = did_fall_down || did_up_jump || y_within_jump;
        let can_skip_x = x_distance < DOUBLE_JUMP_THRESHOLD;

        can_skip_x && can_skip_y
    }
}

/// Updates the [`Player::Moving`] contextual state.
///
/// This state does not perform any movement but acts as coordinator
/// for other movement states. It keeps track of [`PlayerState::unstuck_counter`], avoids
/// state looping and advancing `intermediates` when the current destination is reached.
///
/// It will first transition to [`Player::DoubleJumping`] and [`Player::Adjusting`] for
/// matching `x` of `dest`. Then, [`Player::Grappling`], [`Player::UpJumping`], [`Player::Jumping`]
/// or [`Player::Falling`] for matching `y` of `dest`. (e.g. horizontal then vertical)
///
/// In auto mob or intermediate destination, most of the movement thresholds are relaxed for
/// more fluid movement.
pub fn update_moving_context(
    context: &Context,
    state: &mut PlayerState,
    dest: Point,
    exact: bool,
    intermediates: Option<MovingIntermediates>,
) -> Player {
    state.use_immediate_control_flow = true;
    if state.track_unstucking() {
        return Player::Unstucking(
            Timeout::default(),
            None,
            state.track_unstucking_transitioned(),
        );
    }

    let cur_pos = state.last_known_pos.unwrap();
    let moving = Moving::new(cur_pos, dest, exact, intermediates);
    let is_intermediate = moving.is_destination_intermediate();
    let skip_destination = moving.auto_mob_can_skip_current_destination(state);

    let (x_distance, _) = moving.x_distance_direction_from(true, cur_pos);
    let (y_distance, y_direction) = moving.y_distance_direction_from(true, cur_pos);

    let disable_adjusting = state.config.disable_adjusting;

    // Check to double jump
    if !skip_destination && x_distance >= state.double_jump_threshold(is_intermediate) {
        let require_stationary = state.has_ping_pong_action_only()
            && !matches!(
                state.last_movement,
                Some(LastMovement::Grappling | LastMovement::UpJumping)
            );
        return abort_action_on_state_repeat(
            Player::DoubleJumping(DoubleJumping::new(moving, false, require_stationary)),
            context,
            state,
        );
    }

    // Check to adjust and allow disabling adjusting only if `exact` is false
    if !skip_destination
        && ((!disable_adjusting && x_distance >= ADJUSTING_MEDIUM_THRESHOLD)
            || (exact && x_distance >= ADJUSTING_SHORT_THRESHOLD))
    {
        return abort_action_on_state_repeat(
            Player::Adjusting(Adjusting::new(moving)),
            context,
            state,
        );
    }

    // Check to grapple
    let has_teleport_key = state.config.teleport_key.is_some();
    if !skip_destination
        && y_direction > 0
        && ((!has_teleport_key && y_distance >= GRAPPLING_THRESHOLD)
            || (has_teleport_key && y_distance >= GRAPPLING_MAX_THRESHOLD))
        && !state.should_disable_grappling()
    {
        return abort_action_on_state_repeat(Player::Grappling(moving), context, state);
    }

    // Check to up jump
    if !skip_destination && y_direction > 0 && y_distance >= UP_JUMP_THRESHOLD {
        // In auto mob with platforms pathing and up jump only, immediately aborts the action
        // if there are no intermediate points and the distance is too big to up jump.
        if state.has_auto_mob_action_only()
            && state.config.auto_mob_platforms_pathing
            && state.config.auto_mob_platforms_pathing_up_jump_only
            && intermediates.is_none()
            && y_distance >= GRAPPLING_THRESHOLD
        {
            debug!(target: "player", "auto mob aborted because distance for up jump only is too big");
            state.clear_action_completed();
            return Player::Idle;
        }

        return abort_action_on_state_repeat(
            Player::UpJumping(UpJumping::new(moving)),
            context,
            state,
        );
    }

    // Check to jump
    if !skip_destination && y_direction > 0 && JUMPABLE_RANGE.contains(&y_distance) {
        return abort_action_on_state_repeat(Player::Jumping(moving), context, state);
    }

    // Check to fall
    if !skip_destination
        && y_direction < 0
        && y_distance >= state.falling_threshold(is_intermediate)
    {
        return abort_action_on_state_repeat(
            Player::Falling {
                moving,
                anchor: cur_pos,
                timeout_on_complete: false,
            },
            context,
            state,
        );
    }

    debug!(target: "player", "reached {dest:?} with actual position {cur_pos:?}");
    if let Some(mut intermediates) = intermediates
        && let Some((dest, exact)) = intermediates.next()
    {
        state.clear_unstucking(false);
        state.clear_last_movement();

        if matches!(moving.intermediate_hint(), Some(MovementHint::WalkAndJump)) {
            // TODO: Any better way ???
            state.stalling_timeout_state = Some(Player::Jumping(Moving::new(
                cur_pos,
                dest,
                exact,
                Some(intermediates),
            )));

            let key = if dest.x - cur_pos.x >= 0 {
                KeyKind::Right
            } else {
                KeyKind::Left
            };
            let _ = context.input.send_key_down(key);
            return Player::Stalling(Timeout::default(), 3);
        }

        return Player::Moving(dest, exact, Some(intermediates));
    }

    let last_known_direction = state.last_known_direction;
    on_action(
        state,
        |action| on_player_action(last_known_direction, action, moving),
        || Player::Idle,
    )
}

/// Aborts the action when state starts looping.
///
/// Note: Initially, this is only intended for auto mobbing until rune pathing is added...
#[inline]
fn abort_action_on_state_repeat(
    next: Player,
    context: &Context,
    state: &mut PlayerState,
) -> Player {
    if state.track_last_movement_repeated() {
        info!(target: "player", "abort action due to repeated state");
        state.auto_mob_track_ignore_xs(context, true);
        state.clear_action_completed();
        return Player::Idle;
    }
    next
}

fn on_player_action(
    last_known_direction: ActionKeyDirection,
    action: PlayerAction,
    moving: Moving,
) -> Option<(Player, bool)> {
    match action {
        PlayerAction::Move(Move {
            wait_after_move_ticks,
            ..
        }) => {
            if wait_after_move_ticks > 0 {
                Some((
                    Player::Stalling(Timeout::default(), wait_after_move_ticks),
                    false,
                ))
            } else {
                Some((Player::Idle, true))
            }
        }
        PlayerAction::Key(Key {
            with: ActionKeyWith::DoubleJump,
            direction,
            ..
        }) => {
            if matches!(direction, ActionKeyDirection::Any) || direction == last_known_direction {
                Some((
                    Player::DoubleJumping(DoubleJumping::new(moving, true, false)),
                    false,
                ))
            } else {
                Some((Player::UseKey(UseKey::from_action(action)), false))
            }
        }
        PlayerAction::Key(Key {
            with: ActionKeyWith::Any | ActionKeyWith::Stationary,
            ..
        }) => Some((Player::UseKey(UseKey::from_action(action)), false)),
        PlayerAction::AutoMob(_) => Some((
            Player::UseKey(UseKey::from_action_pos(action, Some(moving.pos))),
            false,
        )),
        PlayerAction::SolveRune => Some((Player::SolvingRune(SolvingRune::default()), false)),
        PlayerAction::PingPong(_) => Some((Player::Idle, true)),
        PlayerAction::Chat(_) | PlayerAction::Panic(_) | PlayerAction::FamiliarsSwap(_) => {
            panic!("unhandled action {action:?}")
        }
    }
}

#[inline]
pub fn find_intermediate_points(
    platforms: &Array<PlatformWithNeighbors, MAX_PLATFORMS_COUNT>,
    cur_pos: Point,
    dest: Point,
    exact: bool,
    up_jump_only: bool,
    enable_hint: bool,
) -> Option<MovingIntermediates> {
    let vertical_threshold = if up_jump_only {
        GRAPPLING_THRESHOLD
    } else {
        GRAPPLING_MAX_THRESHOLD
    };
    let vec = find_points_with(
        platforms,
        cur_pos,
        dest,
        enable_hint,
        DOUBLE_JUMP_THRESHOLD,
        JUMP_THRESHOLD,
        vertical_threshold,
    )?;
    let len = vec.len();
    let array = Array::from_iter(
        vec.into_iter()
            .enumerate()
            .map(|(i, (point, hint))| (point, hint, if i == len - 1 { exact } else { false })),
    );
    Some(MovingIntermediates {
        current: 0,
        inner: array,
    })
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use opencv::core::Point;

    use super::*;
    use crate::player::Player;

    #[test]
    fn update_moving_to_double_jump() {
        let context = Context::new(None, None);
        let mut state = PlayerState::default();
        state.last_known_pos = Some(Point::new(0, 0));

        let dest = Point::new(100, 0); // Large x-distance triggers double jump
        let player = update_moving_context(&context, &mut state, dest, false, None);

        assert_matches!(player, Player::DoubleJumping(_));
    }

    #[test]
    fn update_moving_to_adjusting() {
        let context = Context::new(None, None);
        let mut state = PlayerState::default();
        state.last_known_pos = Some(Point::new(0, 0));

        let dest = Point::new(20, 0); // Less than double jump x-distance
        let player = update_moving_context(&context, &mut state, dest, false, None);

        assert_matches!(player, Player::Adjusting(_));
    }

    #[test]
    fn update_moving_to_grappling() {
        let context = Context::new(None, None);
        let mut state = PlayerState::default();
        state.config.grappling_key = Some(KeyKind::A);
        state.last_known_pos = Some(Point::new(0, 0));

        let dest = Point::new(0, GRAPPLING_THRESHOLD + 10);
        let player = update_moving_context(&context, &mut state, dest, true, None);

        assert_matches!(player, Player::Grappling(_));
    }

    #[test]
    fn update_moving_to_upjump() {
        let context = Context::new(None, None);
        let mut state = PlayerState::default();
        state.last_known_pos = Some(Point::new(0, 0));

        let dest = Point::new(0, 20); // y-distance below grappling
        let player = update_moving_context(&context, &mut state, dest, true, None);

        assert_matches!(player, Player::UpJumping(_));
    }

    #[test]
    fn update_moving_to_jumping() {
        let context = Context::new(None, None);
        let cur_pos = Point::new(100, 100);
        let dest = Point::new(100, 106);
        let mut state = PlayerState::default();
        state.last_known_pos = Some(cur_pos);

        let player = update_moving_context(&context, &mut state, dest, false, None);

        assert_matches!(player, Player::Jumping(_));
    }

    #[test]
    fn update_moving_to_falling() {
        let context = Context::new(None, None);
        let cur_pos = Point::new(100, 100);
        let dest = Point::new(100, 50);
        let mut state = PlayerState::default();
        state.last_known_pos = Some(cur_pos);

        let player = update_moving_context(&context, &mut state, dest, false, None);

        assert_matches!(
            player,
            Player::Falling {
                moving: _,
                anchor: _,
                timeout_on_complete: _
            }
        );
    }

    #[test]
    fn update_moving_to_idle_when_destination_reached() {
        let context = Context::new(None, None);
        let mut state = PlayerState::default();
        let pos = Point::new(100, 200);
        state.last_known_pos = Some(pos);

        let player = update_moving_context(&context, &mut state, pos, true, None);

        assert_matches!(player, Player::Idle);
    }

    #[test]
    fn update_moving_with_intermediate_points_triggers_next_move() {
        let context = Context::new(None, None);
        let mut state = PlayerState::default();
        let pos = Point::new(50, 0);
        state.last_known_pos = Some(pos);

        let intermediates = MovingIntermediates {
            current: 1,
            inner: Array::from_iter([
                (pos, MovementHint::Infer, false),
                (Point::new(100, 0), MovementHint::Infer, true),
            ]),
        };

        let player = update_moving_context(&context, &mut state, pos, true, Some(intermediates));

        assert_matches!(player, Player::Moving(Point { x: 100, y: 0 }, _, _));
    }
}
