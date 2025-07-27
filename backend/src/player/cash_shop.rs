use opencv::core::MatTraitConst;

use super::{
    Player, PlayerState,
    timeout::{Lifecycle, Timeout, next_timeout_lifecycle},
};
use crate::{bridge::KeyKind, bridge::MouseKind, context::Context};

#[derive(Clone, Copy, Debug)]
pub enum CashShop {
    Entering,
    Entered,
    Exitting,
    Exitted,
    Stalling,
}

// TODO: Improve this?
pub fn update_cash_shop_context(
    context: &Context,
    state: &PlayerState,
    timeout: Timeout,
    cash_shop: CashShop,
    failed_to_detect_player: bool,
) -> Player {
    match cash_shop {
        CashShop::Entering => {
            let _ = context.input.send_key(state.config.cash_shop_key);
            let next = if context.detector_unwrap().detect_player_in_cash_shop() {
                CashShop::Entered
            } else {
                CashShop::Entering
            };
            Player::CashShopThenExit(timeout, next)
        }
        CashShop::Entered => {
            // Exit after 10 secs
            match next_timeout_lifecycle(timeout, 305) {
                Lifecycle::Ended => Player::CashShopThenExit(timeout, CashShop::Exitting),
                Lifecycle::Started(timeout) | Lifecycle::Updated(timeout) => {
                    Player::CashShopThenExit(timeout, cash_shop)
                }
            }
        }
        CashShop::Exitting => {
            let next = if context.detector_unwrap().detect_player_in_cash_shop() {
                CashShop::Exitting
            } else {
                CashShop::Exitted
            };
            let size = context.detector_unwrap().mat().size().unwrap();
            let _ = context
                .input
                .send_mouse(size.width / 2, size.height / 2, MouseKind::Click);
            let _ = context.input.send_key(KeyKind::Esc);
            let _ = context.input.send_key(KeyKind::Enter);
            Player::CashShopThenExit(timeout, next)
        }
        CashShop::Exitted => {
            if failed_to_detect_player {
                Player::CashShopThenExit(timeout, cash_shop)
            } else {
                Player::CashShopThenExit(Timeout::default(), CashShop::Stalling)
            }
        }
        CashShop::Stalling => {
            // Return after 3 secs
            match next_timeout_lifecycle(timeout, 90) {
                Lifecycle::Ended => Player::Idle,
                Lifecycle::Started(timeout) | Lifecycle::Updated(timeout) => {
                    Player::CashShopThenExit(timeout, cash_shop)
                }
            }
        }
    }
}
