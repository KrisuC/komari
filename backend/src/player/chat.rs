use crate::{
    bridge::KeyKind,
    context::Context,
    player::{
        Player, PlayerState,
        actions::on_action,
        timeout::{Lifecycle, Timeout, next_timeout_lifecycle},
    },
};

const MAX_RETRY: u32 = 3;

#[derive(Debug, Clone, Copy)]
enum ChattingStage {
    OpeningMenu(Timeout, u32),
    Typing(usize),
    Completing(Timeout, bool),
}

impl Default for ChattingStage {
    fn default() -> Self {
        Self::OpeningMenu(Timeout::default(), 0)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Chatting {
    stage: ChattingStage,
}

impl Chatting {
    #[inline]
    fn stage_opening_menu(self, timeout: Timeout, retry_count: u32) -> Chatting {
        Chatting {
            stage: ChattingStage::OpeningMenu(timeout, retry_count),
        }
    }

    #[inline]
    fn stage_typing(self, index: usize) -> Chatting {
        Chatting {
            stage: ChattingStage::Typing(index),
        }
    }

    #[inline]
    fn stage_completing(self, timeout: Timeout, completed: bool) -> Chatting {
        Chatting {
            stage: ChattingStage::Completing(timeout, completed),
        }
    }
}

pub fn update_chatting_context(
    context: &Context,
    state: &mut PlayerState,
    chatting: Chatting,
) -> Player {
    let chatting = match chatting.stage {
        ChattingStage::OpeningMenu(timeout, retry_count) => {
            update_opening_menu(context, chatting, timeout, retry_count)
        }
        ChattingStage::Typing(index) => update_typing(context, state, chatting, index),
        ChattingStage::Completing(timeout, _) => update_completing(context, chatting, timeout),
    };
    let next = if matches!(chatting.stage, ChattingStage::Completing(_, true)) {
        Player::Idle
    } else {
        Player::Chatting(chatting)
    };

    on_action(
        state,
        |_| Some((next, matches!(next, Player::Idle))),
        || {
            // Force cancel if not initiated from an action
            Player::Idle
        },
    )
}

fn update_opening_menu(
    context: &Context,
    chatting: Chatting,
    timeout: Timeout,
    retry_count: u32,
) -> Chatting {
    match next_timeout_lifecycle(timeout, 35) {
        Lifecycle::Started(timeout) => {
            let _ = context.input.send_key(KeyKind::Enter);
            chatting.stage_opening_menu(timeout, retry_count)
        }
        Lifecycle::Ended => {
            if context.detector_unwrap().detect_chat_menu_opened() {
                chatting.stage_typing(0)
            } else if retry_count < MAX_RETRY {
                chatting.stage_opening_menu(timeout, retry_count + 1)
            } else {
                chatting.stage_completing(timeout, false)
            }
        }
        Lifecycle::Updated(timeout) => chatting.stage_opening_menu(timeout, retry_count),
    }
}

fn update_typing(
    context: &Context,
    state: &PlayerState,
    chatting: Chatting,
    index: usize,
) -> Chatting {
    if !context.input.all_keys_cleared() {
        return chatting.stage_typing(index);
    }

    let Some(key) = state
        .chat_content()
        .and_then(|content| content.chars().nth(index))
        .and_then(to_key_kind)
    else {
        return chatting.stage_completing(Timeout::default(), false);
    };
    let _ = context.input.send_key(key);

    if index + 1 < state.chat_content().expect("has value").chars().count() {
        chatting.stage_typing(index + 1)
    } else {
        let _ = context.input.send_key(KeyKind::Enter);
        chatting.stage_completing(Timeout::default(), false)
    }
}

fn update_completing(context: &Context, chatting: Chatting, timeout: Timeout) -> Chatting {
    match next_timeout_lifecycle(timeout, 35) {
        Lifecycle::Updated(timeout) | Lifecycle::Started(timeout) => {
            chatting.stage_completing(timeout, false)
        }
        Lifecycle::Ended => {
            if context.detector_unwrap().detect_chat_menu_opened() {
                let _ = context.input.send_key(KeyKind::Esc);
            }
            chatting.stage_completing(timeout, true)
        }
    }
}

// TODO: Support non-ASCII characters and ASCII capital characters
#[inline]
fn to_key_kind(character: char) -> Option<KeyKind> {
    match character {
        'A' | 'a' => Some(KeyKind::A),
        'B' | 'b' => Some(KeyKind::B),
        'C' | 'c' => Some(KeyKind::C),
        'D' | 'd' => Some(KeyKind::D),
        'E' | 'e' => Some(KeyKind::E),
        'F' | 'f' => Some(KeyKind::F),
        'G' | 'g' => Some(KeyKind::G),
        'H' | 'h' => Some(KeyKind::H),
        'I' | 'i' => Some(KeyKind::I),
        'J' | 'j' => Some(KeyKind::J),
        'K' | 'k' => Some(KeyKind::K),
        'L' | 'l' => Some(KeyKind::L),
        'M' | 'm' => Some(KeyKind::M),
        'N' | 'n' => Some(KeyKind::N),
        'O' | 'o' => Some(KeyKind::O),
        'P' | 'p' => Some(KeyKind::P),
        'Q' | 'q' => Some(KeyKind::Q),
        'R' | 'r' => Some(KeyKind::R),
        'S' | 's' => Some(KeyKind::S),
        'T' | 't' => Some(KeyKind::T),
        'U' | 'u' => Some(KeyKind::U),
        'V' | 'v' => Some(KeyKind::V),
        'W' | 'w' => Some(KeyKind::W),
        'X' | 'x' => Some(KeyKind::X),
        'Y' | 'y' => Some(KeyKind::Y),
        'Z' | 'z' => Some(KeyKind::Z),

        '0' => Some(KeyKind::Zero),
        '1' => Some(KeyKind::One),
        '2' => Some(KeyKind::Two),
        '3' => Some(KeyKind::Three),
        '4' => Some(KeyKind::Four),
        '5' => Some(KeyKind::Five),
        '6' => Some(KeyKind::Six),
        '7' => Some(KeyKind::Seven),
        '8' => Some(KeyKind::Eight),
        '9' => Some(KeyKind::Nine),

        ' ' => Some(KeyKind::Space),
        '`' | '~' => Some(KeyKind::Tilde),
        '\'' | '"' => Some(KeyKind::Quote),
        ';' => Some(KeyKind::Semicolon),
        ',' => Some(KeyKind::Comma),
        '.' => Some(KeyKind::Period),
        '/' => Some(KeyKind::Slash),
        '\x1B' => Some(KeyKind::Esc), // Escape character

        _ => None,
    }
}

#[cfg(test)]
mod tests {}
