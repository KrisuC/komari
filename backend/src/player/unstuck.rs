use opencv::core::Point;

use super::{
    PlayerState,
    timeout::{Lifecycle, Timeout, next_timeout_lifecycle},
};
use crate::{
    bridge::KeyKind,
    context::Context,
    minimap::Minimap,
    player::{MOVE_TIMEOUT, Player},
    task::{Update, update_detection_task},
};

/// A threshold to consider spamming falling action
///
/// This is when the player is inside the top edge of minimap. At least for higher level maps, this
/// seems rare but one possible map is The Forest Of Earth in Arcana.
const Y_IGNORE_THRESHOLD: i32 = 18;

/// Updates the [`Player::Unstucking`] contextual state
///
/// This state can only be transitioned to when [`PlayerState::unstuck_counter`] reached the fixed
/// threshold or when the player moved into the edges of the minimap.
/// If [`PlayerState::unstuck_consecutive_counter`] has not reached the threshold and the player
/// moved into the left/right/top edges of the minimap, it will try to move
/// out as appropriate. It will also try to press ESC key to exit any dialog.
///
/// Each initial transition to [`Player::Unstucking`] increases
/// the [`PlayerState::unstuck_consecutive_counter`] by one. If the threshold is reached, this
/// state will enter GAMBA mode. And by definition, it means `random bullsh*t go`.
pub fn update_unstucking_context(
    context: &Context,
    state: &mut PlayerState,
    timeout: Timeout,
    has_settings: Option<bool>,
    gamba_mode: bool,
) -> Player {
    let Minimap::Idle(idle) = context.minimap else {
        return Player::Detecting;
    };
    let pos = state
        .last_known_pos
        .map(|pos| Point::new(pos.x, idle.bbox.height - pos.y));
    let gamba_mode = gamba_mode || pos.is_none();

    match next_timeout_lifecycle(timeout, MOVE_TIMEOUT) {
        Lifecycle::Started(timeout) => {
            let has_settings = if !gamba_mode && has_settings.is_none() {
                match update_detection_task(context, 0, &mut state.unstuck_task, move |detector| {
                    Ok(detector.detect_esc_settings())
                }) {
                    Update::Ok(has_settings) => Some(has_settings),
                    Update::Err(_) | Update::Pending => {
                        // Stall until ESC settings detection complete
                        return Player::Unstucking(Timeout::default(), has_settings, gamba_mode);
                    }
                }
            } else {
                None
            };
            if has_settings.unwrap_or_default() || (gamba_mode && context.rng.random_bool(0.5)) {
                let _ = context.input.send_key(KeyKind::Esc);
            }

            let to_right = match (gamba_mode, pos) {
                (true, _) => context.rng.random_bool(0.5),
                (_, Some(Point { y, .. })) if y <= Y_IGNORE_THRESHOLD => {
                    return Player::Unstucking(timeout, has_settings, gamba_mode);
                }
                (_, Some(Point { x, .. })) => x <= idle.bbox.width / 2,
                (_, None) => unreachable!(),
            };
            if to_right {
                let _ = context.input.send_key_down(KeyKind::Right);
            } else {
                let _ = context.input.send_key_up(KeyKind::Left);
            }

            Player::Unstucking(timeout, has_settings, gamba_mode)
        }
        Lifecycle::Ended => {
            let _ = context.input.send_key_up(KeyKind::Right);
            let _ = context.input.send_key_up(KeyKind::Left);

            Player::Detecting
        }
        Lifecycle::Updated(timeout) => {
            let send_space = match (gamba_mode, pos) {
                (true, _) => true,
                (_, Some(pos)) if pos.y > Y_IGNORE_THRESHOLD => true,
                _ => false,
            };
            if send_space {
                let _ = context.input.send_key(state.config.jump_key);
            }

            Player::Unstucking(timeout, has_settings, gamba_mode)
        }
    }
}
