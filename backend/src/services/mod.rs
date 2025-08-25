use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use opencv::{
    core::{ToInputArray, Vector},
    imgcodecs::imencode_def,
};
use platforms::input::InputKind;
use serenity::all::{CreateAttachment, EditInteractionResponse};
use strum::EnumMessage;
use tokio::{
    sync::broadcast::Receiver,
    task::{JoinHandle, spawn},
    time::sleep,
};

use crate::{
    ActionKeyDirection, ActionKeyWith, Character, CycleRunStopMode, GameState, KeyBinding,
    LinkKeyBinding, Minimap, NavigationPath, RequestHandler, RotateKind, Settings,
    bot::{BotAction, BotCommandKind},
    bridge::{Capture, DefaultCapture, DefaultInput, DefaultInputReceiver, InputMethod},
    buff::BuffState,
    context::{Context, ContextEvent, Operation},
    database::Seeds,
    minimap::MinimapState,
    navigator::Navigator,
    notification::NotificationKind,
    player::{
        Chat, ChattingContent, Key, Panic, PanicTo, Panicking, Player, PlayerAction, PlayerState,
    },
    poll_request,
    rotator::Rotator,
    services::{
        bot::BotService,
        character::{CharacterService, DefaultCharacterService},
        game::{DefaultGameService, GameEvent, GameService},
        minimap::{DefaultMinimapService, MinimapService},
        navigator::{DefaultNavigatorService, NavigatorService},
        rotator::{DefaultRotatorService, RotatorService},
        settings::{DefaultSettingsService, SettingsService},
    },
};
#[cfg(debug_assertions)]
use crate::{DebugState, services::debug::DebugService};

mod bot;
mod character;
#[cfg(debug_assertions)]
mod debug;
mod game;
mod minimap;
mod navigator;
mod rotator;
mod settings;

#[derive(Debug)]
pub struct PollArgs<'a> {
    pub context: &'a mut Context,
    pub player: &'a mut PlayerState,
    pub minimap: &'a mut MinimapState,
    pub buffs: &'a mut Vec<BuffState>,
    pub rotator: &'a mut dyn Rotator,
    pub navigator: &'a mut dyn Navigator,
    pub capture: &'a mut dyn Capture,
}

#[derive(Debug)]
pub struct DefaultService {
    event_receiver: Receiver<ContextEvent>,
    pending_halt: Option<JoinHandle<()>>,
    game: Box<dyn GameService>,
    minimap: Box<dyn MinimapService>,
    character: Box<dyn CharacterService>,
    rotator: Box<dyn RotatorService>,
    navigator: Box<dyn NavigatorService>,
    settings: Box<dyn SettingsService>,
    bot: BotService,
    #[cfg(debug_assertions)]
    debug: DebugService,
}

impl DefaultService {
    pub fn new(
        seeds: Seeds,
        settings: Rc<RefCell<Settings>>,
        event_receiver: Receiver<ContextEvent>,
    ) -> (Self, DefaultInput, DefaultCapture) {
        let mut settings_service = DefaultSettingsService::new(settings.clone());

        // Initialize with default window and input method
        let window = settings_service.selected_window();
        let input_method = InputMethod::Default(window, InputKind::Focused);
        let mut input = DefaultInput::new(input_method, seeds);
        let mut input_receiver = DefaultInputReceiver::new(window, InputKind::Focused);

        let mut bot = BotService::default();
        let mut capture = DefaultCapture::new(window);
        // Update to current settings
        settings_service.update_selected_window(
            &mut input,
            &mut input_receiver,
            &mut capture,
            None,
        );
        bot.update(&settings_service.settings());

        let service = Self {
            event_receiver,
            pending_halt: None,
            game: Box::new(DefaultGameService::new(input_receiver)),
            minimap: Box::new(DefaultMinimapService::default()),
            character: Box::new(DefaultCharacterService::default()),
            rotator: Box::new(DefaultRotatorService),
            navigator: Box::new(DefaultNavigatorService),
            settings: Box::new(settings_service),
            bot,
            #[cfg(debug_assertions)]
            debug: DebugService::default(),
        };

        (service, input, capture)
    }

    #[inline]
    pub fn poll(&mut self, args: PollArgs<'_>) {
        let mut handler = DefaultRequestHandler {
            service: self,
            args,
        };
        // TODO: Maybe handling 1 by 1 on each tick instead of all at once?
        handler.poll_request();
        handler.poll_game_events();
        handler.poll_context_event();
        handler.poll_bot();
        handler.broadcast_state();
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

    fn poll_game_events(&mut self) {
        let events = self.service.game.poll_events(
            self.service
                .minimap
                .minimap()
                .and_then(|character| character.id),
            self.service
                .character
                .character()
                .and_then(|character| character.id),
            &self.service.settings.settings(),
        );
        for event in events {
            match event {
                GameEvent::ToggleOperation => {
                    let kind = if self.args.context.operation.halting() {
                        RotateKind::Run
                    } else {
                        RotateKind::Halt
                    };
                    self.update_halting(kind);
                }
                GameEvent::MinimapUpdated(minimap) => {
                    self.on_update_minimap(self.service.minimap.preset(), minimap)
                }
                GameEvent::CharacterUpdated(character) => self.on_update_character(character),
                GameEvent::SettingsUpdated(settings) => {
                    self.service.settings.update(
                        &mut self.args.context.operation,
                        self.args.context.input.as_mut(),
                        self.service.game.input_receiver_mut(),
                        self.args.capture,
                        settings,
                    );
                    self.service.bot.update(&self.service.settings.settings());
                    self.service.rotator.update(
                        self.args.rotator,
                        self.service.minimap.minimap(),
                        self.service.character.character(),
                        &self.service.settings.settings(),
                        self.service.game.actions(),
                        self.service.game.buffs(),
                    );
                }
                GameEvent::NavigationPathsUpdated => self.args.navigator.mark_dirty(true),
            }
        }

        #[cfg(debug_assertions)]
        self.service.debug.poll(self.args.context);
    }

    fn poll_context_event(&mut self) {
        const PENDING_HALT_SECS: u64 = 12;

        if self
            .service
            .pending_halt
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
        {
            self.service.pending_halt = None;
            if !self.args.navigator.was_last_point_available_or_completed() {
                self.update_halt_or_panic(true, true);
            }
        }

        let Some(event) = self.service.event_receiver.try_recv().ok() else {
            return;
        };
        match event {
            ContextEvent::CycledToHalt => {
                self.update_halt_or_panic(false, true);
            }
            ContextEvent::PlayerDied => {
                self.update_halt_or_panic(true, false);
            }
            ContextEvent::MinimapChanged => {
                if self.args.context.operation.halting()
                    | !self.service.settings.settings().stop_on_fail_or_change_map
                {
                    return;
                }

                let player_panicking = matches!(
                    self.args.context.player,
                    Player::Panicking(Panicking {
                        to: PanicTo::Channel,
                        ..
                    })
                );
                if player_panicking {
                    return;
                }
                self.service.pending_halt = Some(spawn(async move {
                    sleep(Duration::from_secs(PENDING_HALT_SECS)).await;
                }));
            }
            ContextEvent::CaptureFailed => {
                if self.args.context.operation.halting() {
                    return;
                }

                if self.service.settings.settings().stop_on_fail_or_change_map {
                    self.update_halt_or_panic(true, false);
                }
                let _ = self
                    .args
                    .context
                    .notification
                    .schedule_notification(NotificationKind::FailOrMapChange);
            }
        }
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
                    if self.service.minimap.minimap().is_none()
                        || self.service.character.character().is_none()
                    {
                        let _ = command.sender.send(
                            EditInteractionResponse::new().content("No map or character data set."),
                        );
                        return;
                    }
                    let _ = command
                        .sender
                        .send(EditInteractionResponse::new().content("Bot started running."));
                    self.on_rotate_actions(RotateKind::Run);
                }
                BotCommandKind::Stop { go_to_town } => {
                    let _ = command
                        .sender
                        .send(EditInteractionResponse::new().content("Bot stopped running."));
                    self.update_halt_or_panic(true, go_to_town);
                }
                BotCommandKind::Status => {
                    let (status, frame) = state_and_frame(self.args.context);
                    let attachment =
                        frame.map(|bytes| CreateAttachment::bytes(bytes, "image.webp"));

                    let mut builder = EditInteractionResponse::new().content(status);
                    if let Some(attachment) = attachment {
                        builder = builder.new_attachment(attachment);
                    }

                    let _ = command.sender.send(builder);
                }
                BotCommandKind::Chat { content } => {
                    if content.chars().count() >= ChattingContent::MAX_LENGTH {
                        let builder = EditInteractionResponse::new().content(format!(
                            "Message length must be less than {} characters.",
                            ChattingContent::MAX_LENGTH
                        ));
                        let _ = command.sender.send(builder);
                        return;
                    }

                    let _ = command
                        .sender
                        .send(EditInteractionResponse::new().content("Queued a chat action."));
                    let action = PlayerAction::Chat(Chat { content });
                    self.args.rotator.inject_action(action);
                }
                BotCommandKind::Action { action, count } => {
                    // Emulate these actions through keys instead to avoid requiring position
                    let player_action = match action {
                        BotAction::Jump => PlayerAction::Key(Key {
                            key: self.args.player.config.jump_key.into(),
                            link_key: None,
                            count,
                            position: None,
                            direction: ActionKeyDirection::Any, // Must always be Any
                            with: ActionKeyWith::Any,           // Must always be Any
                            wait_before_use_ticks: 0,
                            wait_before_use_ticks_random_range: 5,
                            wait_after_use_ticks: 15,
                            wait_after_use_ticks_random_range: 0,
                        }),
                        BotAction::DoubleJump => {
                            PlayerAction::Key(Key {
                                key: self.args.player.config.jump_key.into(),
                                link_key: Some(LinkKeyBinding::Before(
                                    self.args.player.config.jump_key.into(),
                                )),
                                count,
                                position: None,
                                direction: ActionKeyDirection::Any, // Must always be Any
                                with: ActionKeyWith::Any,           // Must always be Any
                                wait_before_use_ticks: 0,
                                wait_before_use_ticks_random_range: 0,
                                wait_after_use_ticks: 0,
                                wait_after_use_ticks_random_range: 55,
                            })
                        }
                        BotAction::Crouch => {
                            PlayerAction::Key(Key {
                                key: KeyBinding::Down,
                                link_key: Some(LinkKeyBinding::Along(KeyBinding::Down)),
                                count,
                                position: None,
                                direction: ActionKeyDirection::Any, // Must always be Any
                                with: ActionKeyWith::Any,           // Must always be Any
                                wait_before_use_ticks: 0,
                                wait_before_use_ticks_random_range: 0,
                                wait_after_use_ticks: 10,
                                wait_after_use_ticks_random_range: 0,
                            })
                        }
                    };
                    self.args.rotator.inject_action(player_action.clone());
                    let _ = command
                        .sender
                        .send(EditInteractionResponse::new().content(format!(
                            "Queued `{}` x {count}",
                            action.get_message().expect("has message")
                        )));
                }
            }
        }
    }

    fn broadcast_state(&self) {
        self.service.game.broadcast_state(
            self.args.context,
            self.args.player,
            self.service.minimap.minimap(),
        );
    }

    fn update_halting(&mut self, kind: RotateKind) {
        let settings = self.service.settings.settings();
        let operation = self.args.context.operation;

        self.args.context.operation = match (kind, settings.cycle_run_stop) {
            (RotateKind::TemporaryHalt, CycleRunStopMode::None) | (RotateKind::Halt, _) => {
                Operation::Halting
            }
            (RotateKind::TemporaryHalt, _) => {
                if let Operation::RunUntil {
                    instant,
                    run_duration_millis,
                    stop_duration_millis,
                    once,
                } = operation
                {
                    Operation::TemporaryHalting {
                        resume: instant.saturating_duration_since(Instant::now()),
                        run_duration_millis,
                        stop_duration_millis,
                        once,
                    }
                } else {
                    Operation::Halting
                }
            }
            (RotateKind::Run, CycleRunStopMode::Once | CycleRunStopMode::Repeat) => {
                if let Operation::TemporaryHalting {
                    resume,
                    run_duration_millis,
                    stop_duration_millis,
                    once,
                } = operation
                {
                    Operation::RunUntil {
                        instant: Instant::now() + resume,
                        run_duration_millis,
                        stop_duration_millis,
                        once,
                    }
                } else {
                    Operation::run_until(
                        settings.cycle_run_duration_millis,
                        settings.cycle_stop_duration_millis,
                        matches!(settings.cycle_run_stop, CycleRunStopMode::Once),
                    )
                }
            }
            (RotateKind::Run, CycleRunStopMode::None) => Operation::Running,
        };
        if matches!(kind, RotateKind::Halt | RotateKind::TemporaryHalt) {
            self.args.rotator.reset_queue();
            self.args.player.clear_actions_aborted(true);
            if let Some(handle) = self.service.pending_halt.take() {
                handle.abort();
            }
        }
    }

    fn update_halt_or_panic(&mut self, should_halt: bool, should_panic: bool) {
        self.args.rotator.reset_queue();
        self.args.player.clear_actions_aborted(!should_panic);
        if should_halt {
            if let Some(handle) = self.service.pending_halt.take() {
                handle.abort();
            }
            self.args.context.operation = Operation::Halting;
        }
        if should_panic {
            self.args
                .rotator
                .inject_action(PlayerAction::Panic(Panic { to: PanicTo::Town }));
        }
    }
}

impl RequestHandler for DefaultRequestHandler<'_> {
    fn on_rotate_actions(&mut self, kind: RotateKind) {
        if self.service.minimap.minimap().is_none() || self.service.character.character().is_none()
        {
            return;
        }
        self.update_halting(kind);
    }

    fn on_create_minimap(&self, name: String) -> Option<Minimap> {
        self.service.minimap.create(self.args.context, name)
    }

    fn on_update_minimap(&mut self, preset: Option<String>, minimap: Option<Minimap>) {
        self.service.minimap.set_minimap_preset(minimap, preset);
        self.service
            .minimap
            .update(self.args.minimap, self.args.player);
        let minimap = self.service.minimap.minimap();
        let character = self.service.character.character();

        self.service
            .game
            .update_actions(minimap, self.service.minimap.preset(), character);

        self.args
            .navigator
            .mark_dirty_with_destination(minimap.and_then(|minimap| minimap.paths_id_index));

        self.service.rotator.update(
            self.args.rotator,
            minimap,
            character,
            &self.service.settings.settings(),
            self.service.game.actions(),
            self.service.game.buffs(),
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

    fn on_navigation_snapshot_as_grayscale(&self, base64: String) -> String {
        self.service
            .navigator
            .navigation_snapshot_as_grayscale(base64)
    }

    fn on_update_character(&mut self, character: Option<Character>) {
        self.service.character.set_character(character);
        self.service.character.update(self.args.player);

        let character = self.service.character.character();
        let minimap = self.service.minimap.minimap();
        let preset = self.service.minimap.preset();
        let settings = self.service.settings.settings();

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
            self.service.game.actions(),
            self.service.game.buffs(),
        );
    }

    fn on_redetect_minimap(&mut self) {
        self.service.minimap.redetect(self.args.context);
        self.args.navigator.mark_dirty(true);
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
            self.service.settings.window_names(),
            self.service.settings.selected_window_index(),
        )
    }

    fn on_select_capture_handle(&mut self, index: Option<usize>) {
        self.service.settings.update_selected_window(
            self.args.context.input.as_mut(),
            self.service.game.input_receiver_mut(),
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

fn state_and_frame(context: &Context) -> (String, Option<Vec<u8>>) {
    let frame = context
        .detector
        .as_ref()
        .and_then(|detector| frame_from(detector.mat()));

    let state = context.player.to_string();
    let operation = match context.operation {
        Operation::HaltUntil { instant, .. } => {
            format!("Halting for {}", duration_from_instant(instant))
        }
        Operation::TemporaryHalting { resume, .. } => format!(
            "Halting temporarily with {} remaining",
            duration_from(resume)
        ),
        Operation::Halting => "Halting".to_string(),
        Operation::Running => "Running".to_string(),
        Operation::RunUntil { instant, .. } => {
            format!("Running for {}", duration_from_instant(instant))
        }
    };
    let info = [
        format!("- State: ``{state}``"),
        format!("- Operation: ``{operation}``"),
    ]
    .join("\n");

    (info, frame)
}

#[inline]
fn duration_from_instant(instant: Instant) -> String {
    duration_from(instant.saturating_duration_since(Instant::now()))
}

#[inline]
fn duration_from(duration: Duration) -> String {
    let seconds = duration.as_secs() % 60;
    let minutes = (duration.as_secs() / 60) % 60;
    let hours = (duration.as_secs() / 60) / 60;

    format!("{hours:0>2}:{minutes:0>2}:{seconds:0>2}")
}

#[inline]
fn frame_from(mat: &impl ToInputArray) -> Option<Vec<u8>> {
    let mut vector = Vector::new();
    imencode_def(".webp", mat, &mut vector).ok()?;
    Some(Vec::from_iter(vector))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use mockall::Sequence;
    use tokio::sync::broadcast::channel;

    use super::*;
    use crate::{
        Action, Character, KeyBindingConfiguration,
        bridge::MockCapture,
        buff::BuffKind,
        context::Context,
        database::Minimap as MinimapData,
        minimap::MinimapState,
        navigator::MockNavigator,
        player::PlayerState,
        rotator::MockRotator,
        services::{
            character::MockCharacterService, game::MockGameService, minimap::MockMinimapService,
            rotator::MockRotatorService, settings::MockSettingsService,
        },
    };

    fn mock_poll_args(
        (context, player, minimap, buffs, rotator, navigator, capture): &mut (
            Context,
            PlayerState,
            MinimapState,
            Vec<BuffState>,
            MockRotator,
            MockNavigator,
            MockCapture,
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
        MockRotator,
        MockNavigator,
        MockCapture,
    ) {
        let context = Context::new(None, None);
        let player = PlayerState::default();
        let minimap = MinimapState::default();
        let buffs = vec![];
        let rotator = MockRotator::default();
        let navigator = MockNavigator::default();
        let capture = MockCapture::default();

        (context, player, minimap, buffs, rotator, navigator, capture)
    }

    #[test]
    fn on_update_minimap_triggers_all_services() {
        let mut states = mock_states();

        let minimap_data = Box::leak(Box::new(MinimapData::default()));
        let character_data = Box::leak(Box::new(Character::default()));
        let settings_data = Box::leak(Box::new(RefCell::new(Settings::default())));
        let actions = Vec::<Action>::new();
        let buffs = Vec::<(BuffKind, KeyBinding)>::new();

        let mut game = MockGameService::default();
        let mut character = MockCharacterService::default();
        let mut minimap = MockMinimapService::default();
        let mut rotator = MockRotatorService::default();
        let navigator = Box::new(DefaultNavigatorService);
        let mut settings = MockSettingsService::default();
        let mut sequence = Sequence::new();

        minimap
            .expect_set_minimap_preset()
            .once()
            .in_sequence(&mut sequence);
        minimap.expect_update().once().in_sequence(&mut sequence);
        minimap
            .expect_minimap()
            .once()
            .return_const(Some(&*minimap_data))
            .in_sequence(&mut sequence);

        character
            .expect_character()
            .once()
            .return_const(Some(&*character_data))
            .in_sequence(&mut sequence);

        minimap
            .expect_preset()
            .once()
            .return_const(Some("preset".to_string()))
            .in_sequence(&mut sequence);

        game.expect_update_actions()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);

        states
            .5
            .expect_mark_dirty_with_destination()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);
        settings
            .expect_settings()
            .once()
            .returning_st(|| settings_data.borrow());
        game.expect_actions().once().return_const(actions);
        game.expect_buffs().once().return_const(buffs);
        rotator
            .expect_update()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);

        let (_tx, rx) = channel(1);
        let args = mock_poll_args(&mut states);
        let mut service = DefaultService {
            event_receiver: rx,
            pending_halt: None,
            game: Box::new(game),
            minimap: Box::new(minimap),
            character: Box::new(character),
            rotator: Box::new(rotator),
            navigator,
            settings: Box::new(settings),
            bot: BotService::default(),
            #[cfg(debug_assertions)]
            debug: crate::services::debug::DebugService::default(),
        };
        let mut handler = DefaultRequestHandler {
            service: &mut service,
            args,
        };

        handler.on_update_minimap(Some("preset".into()), Some(minimap_data.clone()));
    }

    #[test]
    fn on_update_character_calls_dependencies() {
        let mut states = mock_states();
        states.3.push(BuffState::new(BuffKind::Familiar));
        states.3.push(BuffState::new(BuffKind::SayramElixir));

        let args = mock_poll_args(&mut states);
        let minimap_data = Box::leak(Box::new(MinimapData::default()));
        let character_data = Box::leak(Box::new(Character {
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
        let settings_data = Box::leak(Box::new(RefCell::new(Settings::default())));
        let actions = Vec::<Action>::new();
        let buffs = Vec::<(BuffKind, KeyBinding)>::new();

        let mut game = MockGameService::default();
        let mut character = MockCharacterService::default();
        let mut minimap = MockMinimapService::default();
        let mut rotator = MockRotatorService::default();
        let navigator = Box::new(DefaultNavigatorService);
        let mut settings = MockSettingsService::default();
        let mut sequence = Sequence::new();

        character
            .expect_set_character()
            .once()
            .in_sequence(&mut sequence);
        character.expect_update().once().in_sequence(&mut sequence);

        character
            .expect_character()
            .once()
            .return_const(Some(&*character_data))
            .in_sequence(&mut sequence);
        minimap
            .expect_minimap()
            .once()
            .return_const(Some(&*minimap_data))
            .in_sequence(&mut sequence);
        minimap
            .expect_preset()
            .once()
            .return_const(Some("preset".to_string()))
            .in_sequence(&mut sequence);
        settings
            .expect_settings()
            .once()
            .returning_st(|| settings_data.borrow());

        game.expect_update_actions()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);
        game.expect_update_buffs()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);

        game.expect_actions()
            .once()
            .return_const(actions)
            .in_sequence(&mut sequence);
        game.expect_buffs()
            .once()
            .return_const(buffs)
            .in_sequence(&mut sequence);
        rotator
            .expect_update()
            .once()
            .return_const(())
            .in_sequence(&mut sequence);

        let (_tx, rx) = channel(1);
        let mut service = DefaultService {
            event_receiver: rx,
            pending_halt: None,
            game: Box::new(game),
            minimap: Box::new(minimap),
            character: Box::new(character),
            rotator: Box::new(rotator),
            navigator,
            settings: Box::new(settings),
            bot: BotService::default(),
            #[cfg(debug_assertions)]
            debug: crate::services::debug::DebugService::default(),
        };
        let mut handler = DefaultRequestHandler {
            service: &mut service,
            args,
        };

        handler.on_update_character(Some(character_data.clone()));

        // TODO: Assert buffs
    }
}
