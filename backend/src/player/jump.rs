use super::{
    Player, PlayerState,
    moving::{MOVE_TIMEOUT, Moving},
    state::LastMovement,
    timeout::{ChangeAxis, MovingLifecycle, next_moving_lifecycle_with_axis},
};
use crate::context::Context;

const TIMEOUT: u32 = MOVE_TIMEOUT + 3;

pub fn update_jumping_context(
    context: &Context,
    state: &mut PlayerState,
    moving: Moving,
) -> Player {
    match next_moving_lifecycle_with_axis(
        moving,
        state.last_known_pos.expect("in positional context"),
        TIMEOUT,
        ChangeAxis::Vertical,
    ) {
        MovingLifecycle::Started(moving) => {
            state.last_movement = Some(LastMovement::Jumping);
            let _ = context.input.send_key(state.config.jump_key);
            Player::Jumping(moving)
        }
        MovingLifecycle::Ended(moving) => {
            Player::Moving(moving.dest, moving.exact, moving.intermediates)
        }
        MovingLifecycle::Updated(moving) => Player::Jumping(moving),
    }
}
