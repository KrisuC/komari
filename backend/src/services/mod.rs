use std::{cell::RefCell, rc::Rc};

use mockall_double::double;
use platforms::input::InputKind;
use serenity::all::{CreateAttachment, EditInteractionResponse};
use tokio::sync::broadcast::Receiver;

use crate::{
    Character, GameState, KeyBinding, Minimap, NavigationPath, RequestHandler, Settings,
    bot::BotCommandKind,
    bridge::{DefaultInput, Input, InputMethod},
    buff::BuffState,
    context::{Context, Operation},
    database::Seeds,
    minimap::MinimapState,
    player::{PanicTo, Panicking, Player, PlayerAction, PlayerState},
    poll_request,
    services::{bot::BotService, game::GameEvent},
};
#[cfg(debug_assertions)]
use crate::{DebugState, services::debug::DebugService};
#[double]
use crate::{
    bridge::{Capture, InputReceiver},
    navigator::Navigator,
    rotator::Rotator,
    services::{
        game::GameService, minimap::MinimapService, navigator::NavigatorService,
        player::PlayerService, rotator::RotatorService, settings::SettingsService,
    },
};

mod bot;
#[cfg(debug_assertions)]
mod debug;
mod game;
mod minimap;
mod navigator;
mod player;
mod rotator;
mod settings;

#[derive(Debug)]
pub struct PollArgs<'a> {
    pub context: &'a mut Context,
    pub player: &'a mut PlayerState,
    pub minimap: &'a mut MinimapState,
    pub buffs: &'a mut Vec<BuffState>,
    pub rotator: &'a mut Rotator,
    pub navigator: &'a mut Navigator,
    pub capture: &'a mut Capture,
}

#[derive(Debug)]
pub struct DefaultService {
    game: GameService,
    minimap: MinimapService,
    player: PlayerService,
    rotator: RotatorService,
    navigator: NavigatorService,
    settings: SettingsService,
    bot: BotService,
    #[cfg(debug_assertions)]
    debug: DebugService,
}

impl DefaultService {
    pub fn new(seeds: Seeds, settings: Rc<RefCell<Settings>>) -> (Self, Box<dyn Input>, Capture) {
        let mut settings_service = SettingsService::new(settings.clone());

        // Initialize with default window and input method
        let window = settings_service.current_window();
        let input_method = InputMethod::Default(window, InputKind::Focused);
        let mut input = DefaultInput::new(input_method, seeds);
        let mut input_receiver = InputReceiver::new(window, InputKind::Focused);

        let mut bot = BotService::default();
        let mut capture = Capture::new(window);
        // Update to current settings
        settings_service.update_selected_window(
            &mut input,
            &mut input_receiver,
            &mut capture,
            None,
        );
        bot.update(&settings_service.current());

        let service = Self {
            game: GameService::new(input_receiver),
            minimap: MinimapService::default(),
            player: PlayerService::default(),
            #[allow(clippy::default_constructed_unit_structs)]
            rotator: RotatorService::default(),
            #[allow(clippy::default_constructed_unit_structs)]
            navigator: NavigatorService::default(),
            settings: settings_service,
            bot,
            #[cfg(debug_assertions)]
            debug: DebugService::default(),
        };

        (service, Box::new(input), capture)
    }

    #[inline]
    pub fn poll(&mut self, args: PollArgs<'_>) {
        let mut handler = DefaultRequestHandler {
            service: self,
            args,
        };
        handler.poll_request();
        handler.poll_events();
        handler.poll_bot();
        handler.broadcast_state();
    }

    #[inline]
    pub fn has_minimap_data(&self) -> bool {
        self.minimap.current().is_some()
    }
}

#[inline]
pub fn update_operation_with_halt_or_panic(
    context: &mut Context,
    rotator: &mut Rotator,
    player: &mut PlayerState,
    should_halt: bool,
    should_panic: bool,
) {
    rotator.reset_queue();
    player.clear_actions_aborted(!should_panic);
    if should_halt {
        context.operation = Operation::Halting;
    }
    if should_panic {
        context.player = Player::Panicking(Panicking::new(PanicTo::Town));
    }
}

#[derive(Debug)]
struct DefaultRequestHandler<'a> {
    service: &'a mut DefaultService,
    args: PollArgs<'a>,
}

impl DefaultRequestHandler<'_> {
    fn poll_request(&mut self) {
        poll_request(self);
    }

    fn poll_events(&mut self) {
        let events = self.service.game.poll_events(
            self.service
                .minimap
                .current()
                .and_then(|character| character.id),
            self.service
                .player
                .current()
                .and_then(|character| character.id),
            &self.service.settings.current(),
        );
        for event in events {
            match event {
                GameEvent::ToggleOperation => {
                    let halting = !self.args.context.operation.halting();
                    self.on_rotate_actions(halting);
                }
                GameEvent::MinimapUpdated(minimap) => {
                    self.on_update_minimap(self.service.minimap.current_preset(), minimap)
                }
                GameEvent::CharacterUpdated(character) => self.on_update_character(character),
                GameEvent::SettingsUpdated(settings) => {
                    self.service.settings.update(
                        &mut self.args.context.operation,
                        self.args.context.input.as_mut(),
                        self.service.game.current_input_receiver_mut(),
                        self.args.capture,
                        settings,
                    );
                    self.service.bot.update(&self.service.settings.current());
                    self.service.rotator.update(
                        self.args.rotator,
                        self.service.minimap.current(),
                        self.service.player.current(),
                        &self.service.settings.current(),
                        self.service.game.current_actions(),
                        self.service.game.current_buffs(),
                    );
                }
                GameEvent::NavigationPathsUpdated => self.args.navigator.mark_dirty(true),
            }
        }

        #[cfg(debug_assertions)]
        self.service.debug.poll(self.args.context);
    }

    fn poll_bot(&mut self) {
        if let Some(command) = self.service.bot.poll() {
            match command.kind {
                BotCommandKind::Start => {
                    if !self.args.context.operation.halting() {
                        let _ = command
                            .sender
                            .send(EditInteractionResponse::new().content("Bot already running."));
                        return;
                    }
                    if !self.service.has_minimap_data() || self.service.player.current().is_none() {
                        let _ = command.sender.send(
                            EditInteractionResponse::new().content("No map or character data set."),
                        );
                        return;
                    }
                    let _ = command
                        .sender
                        .send(EditInteractionResponse::new().content("Bot started running."));
                    self.on_rotate_actions(false);
                }
                BotCommandKind::Stop => {
                    let go_to_town = command
                        .options
                        .into_iter()
                        .next()
                        .and_then(|option| option.value.as_bool())
                        .unwrap_or_default();
                    let _ = command
                        .sender
                        .send(EditInteractionResponse::new().content("Bot stopped running."));
                    self.update_operation_halt_or_panic(true, go_to_town);
                }
                BotCommandKind::Status => {
                    let (status, frame) = self.service.game.get_state_and_frame(self.args.context);
                    let attachment = frame.map(|bytes| CreateAttachment::bytes(bytes, "image.png"));

                    let mut builder = EditInteractionResponse::new().content(status);
                    if let Some(attachment) = attachment {
                        builder = builder.new_attachment(attachment);
                    }

                    let _ = command.sender.send(builder);
                }
                BotCommandKind::Chat => {
                    let Some(content) = command
                        .options
                        .into_iter()
                        .next()
                        .and_then(|option| Some(option.value.as_str()?.to_string()))
                    else {
                        return;
                    };
                    let _ = command
                        .sender
                        .send(EditInteractionResponse::new().content("Queued a chat action."));
                    let is_halting = self.args.context.operation.halting();

                    self.args.player.set_chat_content(content);
                    if is_halting {
                        self.args
                            .player
                            .set_priority_action(None, PlayerAction::Chatting);
                    } else {
                        self.args.rotator.inject_action(PlayerAction::Chatting);
                    }
                }
            }
        }
    }

    fn broadcast_state(&self) {
        self.service.game.broadcast_state(
            self.args.context,
            self.args.player,
            self.service.minimap.current(),
        );
    }

    fn update_operation_halt_or_panic(&mut self, should_halt: bool, should_panic: bool) {
        update_operation_with_halt_or_panic(
            self.args.context,
            self.args.rotator,
            self.args.player,
            should_halt,
            should_panic,
        );
    }
}

impl RequestHandler for DefaultRequestHandler<'_> {
    fn on_rotate_actions(&mut self, halting: bool) {
        if self.service.minimap.current().is_none() || self.service.player.current().is_none() {
            return;
        }
        self.service.game.update_operation(
            &mut self.args.context.operation,
            self.args.rotator,
            self.args.player,
            &self.service.settings.current(),
            halting,
        );
    }

    fn on_create_minimap(&self, name: String) -> Option<Minimap> {
        self.service.minimap.create(self.args.context, name)
    }

    fn on_update_minimap(&mut self, preset: Option<String>, minimap: Option<Minimap>) {
        self.service
            .minimap
            .update(self.args.minimap, preset, minimap);
        let minimap = self.service.minimap.current();
        let character = self.service.player.current();

        self.service
            .player
            .update_from_minimap(self.args.player, minimap);

        self.service
            .game
            .update_actions(minimap, self.service.minimap.current_preset(), character);

        self.args
            .navigator
            .mark_dirty_with_destination(minimap.and_then(|minimap| minimap.paths_id_index));

        self.service.rotator.update(
            self.args.rotator,
            minimap,
            character,
            &self.service.settings.current(),
            self.service.game.current_actions(),
            self.service.game.current_buffs(),
        );
    }

    fn on_create_navigation_path(&self) -> Option<NavigationPath> {
        self.service.navigator.create_path(self.args.context)
    }

    fn on_recapture_navigation_path(&self, path: NavigationPath) -> NavigationPath {
        self.service
            .navigator
            .recapture_path(self.args.context, path)
    }

    fn on_update_character(&mut self, character: Option<Character>) {
        self.service.player.update(character);
        self.service.player.update_from_character(self.args.player);

        let character = self.service.player.current();
        let minimap = self.service.minimap.current();
        let preset = self.service.minimap.current_preset();
        let settings = self.service.settings.current();

        self.service.game.update_actions(minimap, preset, character);
        self.service.game.update_buffs(character);
        if let Some(character) = character {
            self.args.buffs.iter_mut().for_each(|state| {
                state.update_enabled_state(character, &settings);
            });
        }
        self.service.rotator.update(
            self.args.rotator,
            minimap,
            character,
            &settings,
            self.service.game.current_actions(),
            self.service.game.current_buffs(),
        );
    }

    fn on_redetect_minimap(&mut self) {
        self.service.minimap.redetect(self.args.context);
    }

    fn on_game_state_receiver(&self) -> Receiver<GameState> {
        self.service.game.subscribe_state()
    }

    fn on_key_receiver(&self) -> Receiver<KeyBinding> {
        self.service.game.subscribe_key()
    }

    fn on_refresh_capture_handles(&mut self) {
        self.service.settings.update_windows();
        self.on_select_capture_handle(None);
    }

    fn on_query_capture_handles(&self) -> (Vec<String>, Option<usize>) {
        (
            self.service.settings.current_window_names(),
            self.service.settings.current_selected_window_index(),
        )
    }

    fn on_select_capture_handle(&mut self, index: Option<usize>) {
        self.service.settings.update_selected_window(
            self.args.context.input.as_mut(),
            self.service.game.current_input_receiver_mut(),
            self.args.capture,
            index,
        );
    }

    #[cfg(debug_assertions)]
    fn on_debug_state_receiver(&self) -> Receiver<DebugState> {
        self.service.debug.subscribe_state()
    }

    #[cfg(debug_assertions)]
    fn on_auto_save_rune(&self, auto_save: bool) {
        self.service
            .debug
            .set_auto_save_rune(self.args.context, auto_save);
    }

    #[cfg(debug_assertions)]
    fn on_capture_image(&self, is_grayscale: bool) {
        self.service
            .debug
            .capture_image(self.args.context, is_grayscale);
    }

    #[cfg(debug_assertions)]
    fn on_infer_rune(&mut self) {
        self.service.debug.infer_rune();
    }

    #[cfg(debug_assertions)]
    fn on_infer_minimap(&self) {
        self.service.debug.infer_minimap(self.args.context);
    }

    #[cfg(debug_assertions)]
    fn on_record_images(&mut self, start: bool) {
        self.service.debug.record_images(start);
    }

    #[cfg(debug_assertions)]
    fn on_test_spin_rune(&self) {
        self.service.debug.test_spin_rune();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use mockall::Sequence;

    use super::*;
    use crate::{
        Action, Character, KeyBindingConfiguration, buff::BuffKind, context::Context,
        database::Minimap as MinimapData, minimap::MinimapState, player::PlayerState,
    };

    fn mock_poll_args(
        (context, player, minimap, buffs, rotator, navigator, capture): &mut (
            Context,
            PlayerState,
            MinimapState,
            Vec<BuffState>,
            Rotator,
            Navigator,
            Capture,
        ),
    ) -> PollArgs<'_> {
        PollArgs {
            context,
            player,
            minimap,
            buffs,
            rotator,
            navigator,
            capture,
        }
    }

    fn mock_states() -> (
        Context,
        PlayerState,
        MinimapState,
        Vec<BuffState>,
        Rotator,
        Navigator,
        Capture,
    ) {
        let context = Context::new(None, None);
        let player = PlayerState::default();
        let minimap = MinimapState::default();
        let buffs = vec![];
        let rotator = Rotator::default();
        let navigator = Navigator::default();
        let capture = Capture::default();

        (context, player, minimap, buffs, rotator, navigator, capture)
    }

    fn mock_service() -> DefaultService {
        let game = GameService::default();
        let player = PlayerService::default();
        let minimap = MinimapService::default();
        let rotator = RotatorService::default();
        let navigator = NavigatorService::default();
        let settings = SettingsService::default();

        DefaultService {
            game,
            minimap,
            player,
            rotator,
            navigator,
            settings,
            bot: BotService::default(),
            #[cfg(debug_assertions)]
            debug: crate::services::debug::DebugService::default(),
        }
    }

    #[test]
    fn on_update_minimap_triggers_all_services() {
        let mut service = mock_service();
        let mut states = mock_states();
        let mut sequence = Sequence::new();
        let args = mock_poll_args(&mut states);
        let mut handler = DefaultRequestHandler {
            service: &mut service,
            args,
        };
        let minimap = Box::leak(Box::new(MinimapData::default()));
        let character = Box::leak(Box::new(Character::default()));
        let settings = Box::leak(Box::new(RefCell::new(Settings::default())));
        let actions = Vec::<Action>::new();
        let buffs = Vec::<(BuffKind, KeyBinding)>::new();

        handler
            .service
            .minimap
            .expect_update()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);
        handler
            .service
            .minimap
            .expect_current()
            .once()
            .return_const(Some(&*minimap))
            .in_sequence(&mut sequence);
        handler
            .service
            .player
            .expect_current()
            .once()
            .return_const(Some(&*character))
            .in_sequence(&mut sequence);
        handler
            .service
            .player
            .expect_update_from_minimap()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);
        handler
            .service
            .minimap
            .expect_current_preset()
            .once()
            .return_const(Some("preset".to_string()))
            .in_sequence(&mut sequence);
        handler
            .service
            .game
            .expect_update_actions()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);
        handler
            .args
            .navigator
            .expect_mark_dirty_with_destination()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);
        handler
            .service
            .settings
            .expect_current()
            .once()
            .returning_st(|| settings.borrow());
        handler
            .service
            .game
            .expect_current_actions()
            .once()
            .return_const(actions);
        handler
            .service
            .game
            .expect_current_buffs()
            .once()
            .return_const(buffs);
        handler
            .service
            .rotator
            .expect_update()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);

        handler.on_update_minimap(Some("preset".into()), Some(minimap.clone()));
    }

    #[test]
    fn on_update_character_calls_dependencies() {
        let mut service = mock_service();
        let mut states = mock_states();
        states.3.push(BuffState::new(BuffKind::Familiar));
        states.3.push(BuffState::new(BuffKind::SayramElixir));

        let mut sequence = Sequence::new();
        let args = mock_poll_args(&mut states);
        let mut handler = DefaultRequestHandler {
            service: &mut service,
            args,
        };
        let minimap = Box::leak(Box::new(MinimapData::default()));
        let character = Box::leak(Box::new(Character {
            sayram_elixir_key: KeyBindingConfiguration {
                key: KeyBinding::C,
                enabled: true,
            },
            familiar_buff_key: KeyBindingConfiguration {
                key: KeyBinding::B,
                enabled: true,
            },
            ..Default::default()
        }));
        let settings = Box::leak(Box::new(RefCell::new(Settings::default())));
        let actions = Vec::<Action>::new();
        let buffs = Vec::<(BuffKind, KeyBinding)>::new();

        handler
            .service
            .player
            .expect_update()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);
        handler
            .service
            .player
            .expect_update_from_character()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);

        handler
            .service
            .player
            .expect_current()
            .once()
            .return_const(Some(&*character))
            .in_sequence(&mut sequence);
        handler
            .service
            .minimap
            .expect_current()
            .once()
            .return_const(Some(&*minimap))
            .in_sequence(&mut sequence);
        handler
            .service
            .minimap
            .expect_current_preset()
            .once()
            .return_const(Some("preset".to_string()))
            .in_sequence(&mut sequence);
        handler
            .service
            .settings
            .expect_current()
            .once()
            .returning_st(|| settings.borrow());

        handler
            .service
            .game
            .expect_update_actions()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);
        handler
            .service
            .game
            .expect_update_buffs()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);

        handler
            .service
            .game
            .expect_current_actions()
            .once()
            .return_const(actions)
            .in_sequence(&mut sequence);
        handler
            .service
            .game
            .expect_current_buffs()
            .once()
            .return_const(buffs)
            .in_sequence(&mut sequence);
        handler
            .service
            .rotator
            .expect_update()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);

        handler.on_update_character(Some(character.clone()));

        // TODO: Assert buffs
    }
}
