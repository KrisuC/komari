use crate::{
    array::Array,
    bridge::KeyKind,
    context::Context,
    player::{
        Player, PlayerState,
        actions::on_action,
        timeout::{Lifecycle, Timeout, next_timeout_lifecycle},
    },
};

const MAX_RETRY: u32 = 3;
const MAX_CONTENT_LENGTH: usize = 256;

pub type ChattingContent = Array<char, MAX_CONTENT_LENGTH>;

impl ChattingContent {
    pub const MAX_LENGTH: usize = MAX_CONTENT_LENGTH;

    #[inline]
    pub fn from_string(content: String) -> ChattingContent {
        ChattingContent::from_iter(content.into_chars())
    }
}

#[derive(Debug, Clone, Copy)]
enum ChattingStage {
    OpeningMenu(Timeout, u32),
    Typing(Timeout, usize),
    Completing(Timeout, bool),
}

#[derive(Debug, Clone, Copy)]
pub struct Chatting {
    stage: ChattingStage,
    content: ChattingContent,
}

impl Chatting {
    pub fn new(content: ChattingContent) -> Self {
        Self {
            stage: ChattingStage::OpeningMenu(Timeout::default(), 0),
            content,
        }
    }

    #[inline]
    fn stage_opening_menu(self, timeout: Timeout, retry_count: u32) -> Chatting {
        Chatting {
            stage: ChattingStage::OpeningMenu(timeout, retry_count),
            ..self
        }
    }

    #[inline]
    fn stage_typing(self, timeout: Timeout, index: usize) -> Chatting {
        Chatting {
            stage: ChattingStage::Typing(timeout, index),
            ..self
        }
    }

    #[inline]
    fn stage_completing(self, timeout: Timeout, completed: bool) -> Chatting {
        Chatting {
            stage: ChattingStage::Completing(timeout, completed),
            ..self
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
        ChattingStage::Typing(timeout, index) => update_typing(context, chatting, timeout, index),
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
                chatting.stage_typing(Timeout::default(), 0)
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
    chatting: Chatting,
    timeout: Timeout,
    index: usize,
) -> Chatting {
    match next_timeout_lifecycle(timeout, 3) {
        Lifecycle::Started(timeout) | Lifecycle::Updated(timeout) => {
            chatting.stage_typing(timeout, index)
        }
        Lifecycle::Ended => {
            let Some(key) = chatting
                .content
                .as_slice()
                .get(index)
                .copied()
                .and_then(to_key_kind)
            else {
                return chatting.stage_completing(Timeout::default(), false);
            };
            let _ = context.input.send_key(key);

            if index + 1 < chatting.content.len() {
                chatting.stage_typing(Timeout::default(), index + 1)
            } else {
                let _ = context.input.send_key(KeyKind::Enter);
                chatting.stage_completing(Timeout::default(), false)
            }
        }
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

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use mockall::predicate::eq;

    use super::*;
    use crate::{bridge::MockInput, context::Context, detect::MockDetector};

    #[test]
    fn update_opening_menu_detects_chat_menu_and_transitions_to_typing() {
        let mut detector = MockDetector::default();
        detector.expect_detect_chat_menu_opened().returning(|| true);

        let context = Context::new(None, Some(detector));
        let chatting = Chatting::new(Array::new());
        let timeout = Timeout {
            current: 35,
            started: true,
            ..Default::default()
        };

        let result = update_opening_menu(&context, chatting, timeout, 0);
        assert_matches!(result.stage, ChattingStage::Typing(_, 0));
    }

    #[test]
    fn update_opening_menu_retries_when_chat_menu_not_opened() {
        let mut detector = MockDetector::default();
        detector
            .expect_detect_chat_menu_opened()
            .returning(|| false);

        let context = Context::new(None, Some(detector));
        let chatting = Chatting::new(Array::new());
        let timeout = Timeout {
            current: 35,
            started: true,
            ..Default::default()
        };

        let result = update_opening_menu(&context, chatting, timeout, 1);
        assert_matches!(result.stage, ChattingStage::OpeningMenu(_, 2));
    }

    #[test]
    fn update_opening_menu_fails_after_max_retries() {
        let mut detector = MockDetector::default();
        detector
            .expect_detect_chat_menu_opened()
            .returning(|| false);

        let context = Context::new(None, Some(detector));
        let chatting = Chatting::new(Array::new());
        let timeout = Timeout {
            current: 35,
            started: true,
            ..Default::default()
        };

        let result = update_opening_menu(&context, chatting, timeout, MAX_RETRY);
        assert_matches!(result.stage, ChattingStage::Completing(_, false));
    }

    #[test]
    fn update_typing_sends_character_key_and_progresses() {
        let mut keys = MockInput::default();
        keys.expect_send_key()
            .once()
            .with(eq(KeyKind::A))
            .returning(|_| Ok(()));
        keys.expect_send_key()
            .once()
            .with(eq(KeyKind::B))
            .returning(|_| Ok(()));
        keys.expect_send_key()
            .once()
            .with(eq(KeyKind::C))
            .returning(|_| Ok(()));
        let context = Context::new(Some(keys), None);
        let mut chatting = Chatting::new(Array::from_iter(['a', 'b', 'c', 'd']));

        for i in 0..3 {
            chatting = update_typing(
                &context,
                chatting,
                Timeout {
                    current: 3,
                    started: true,
                    ..Default::default()
                },
                i,
            );
            assert_matches!(chatting.stage, ChattingStage::Typing(_, index) if index == i + 1);
        }
    }

    #[test]
    fn update_typing_finishes_after_last_character() {
        let mut keys = MockInput::default();
        keys.expect_send_key()
            .once()
            .with(eq(KeyKind::A))
            .returning(|_| Ok(()));
        keys.expect_send_key()
            .once()
            .with(eq(KeyKind::Enter))
            .returning(|_| Ok(()));
        let context = Context::new(Some(keys), None);

        let chatting = Chatting::new(Array::from_iter(['a']));
        let result = update_typing(
            &context,
            chatting,
            Timeout {
                current: 3,
                started: true,
                ..Default::default()
            },
            0,
        );
        assert_matches!(result.stage, ChattingStage::Completing(_, false));
    }

    #[test]
    fn update_typing_completes_if_char_not_found() {
        let context = Context::new(None, None);
        let chatting = Chatting::new(Array::new());

        let result = update_typing(
            &context,
            chatting,
            Timeout {
                current: 3,
                started: true,
                ..Default::default()
            },
            0,
        );
        assert_matches!(result.stage, ChattingStage::Completing(_, false));
    }

    #[test]
    fn update_completing_sends_esc_if_menu_open() {
        let mut detector = MockDetector::default();
        detector.expect_detect_chat_menu_opened().returning(|| true);

        let mut keys = MockInput::default();
        keys.expect_send_key()
            .once()
            .with(eq(KeyKind::Esc))
            .returning(|_| Ok(()));

        let context = Context::new(Some(keys), Some(detector));
        let chatting = Chatting::new(Array::new());

        let result = update_completing(
            &context,
            chatting,
            Timeout {
                current: 35,
                started: true,
                ..Default::default()
            },
        );
        assert_matches!(result.stage, ChattingStage::Completing(_, true));
    }
}
