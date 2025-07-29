use std::{cell::RefCell, rc::Rc};

use mockall_double::double;
use platforms::input::InputKind;
use tokio::sync::broadcast::Receiver;

#[double]
use crate::rotator::Rotator;
#[cfg(debug_assertions)]
use crate::services::debug::DebugService;
use crate::{
    Character, GameState, KeyBinding, Minimap, NavigationPath, RequestHandler, Settings,
    bridge::{Capture, DefaultInput, Input, InputMethod, InputReceiver},
    buff::BuffState,
    context::Context,
    database::Seeds,
    minimap::MinimapState,
    navigator::Navigator,
    player::PlayerState,
    poll_request,
    services::{
        game::{GameEvent, GameService},
        minimap::MinimapService,
        navigator::NavigatorService,
        player::PlayerService,
        rotator::RotatorService,
        settings::SettingsService,
    },
};

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

        let mut capture = Capture::new(window);
        // Update to current settings
        settings_service.update_selected_window(
            &mut input,
            &mut input_receiver,
            &mut capture,
            None,
        );

        let service = Self {
            game: GameService::new(input_receiver),
            minimap: MinimapService::default(),
            player: PlayerService::default(),
            rotator: RotatorService,
            navigator: NavigatorService,
            settings: settings_service,
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
        handler.broadcast_state();
    }

    #[inline]
    pub fn has_minimap_data(&self) -> bool {
        self.minimap.current().is_some()
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
                    self.service.game.update_operation(
                        &mut self.args.context.operation,
                        self.args.rotator,
                        self.args.player,
                        &self.service.settings.current(),
                        halting,
                    )
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
                    self.service.rotator.update(
                        self.args.rotator,
                        self.service.minimap.current(),
                        self.service.player.current(),
                        &self.service.settings.current(),
                        self.service.game.current_actions(),
                        self.service.game.current_buffs(),
                    );
                }
                GameEvent::NavigationPathsUpdated => self.args.navigator.mark_dirty(),
            }
        }

        #[cfg(debug_assertions)]
        self.service.debug.poll(self.args.context);
    }

    fn broadcast_state(&self) {
        self.service.game.broadcast_state(
            self.args.context,
            self.args.player,
            self.service.minimap.current(),
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
        self.service
            .player
            .update_from_minimap(self.args.player, self.service.minimap.current());
        self.service.game.update_actions(
            self.service.minimap.current(),
            self.service.minimap.current_preset(),
            self.service.player.current(),
        );
        self.args.navigator.mark_dirty_with_destination(
            self.service
                .minimap
                .current()
                .and_then(|minimap| minimap.paths_id_index),
        );
        self.service.rotator.update(
            self.args.rotator,
            self.service.minimap.current(),
            self.service.player.current(),
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
        self.service.game.update_actions(
            self.service.minimap.current(),
            self.service.minimap.current_preset(),
            self.service.player.current(),
        );
        self.service
            .game
            .update_buffs(self.service.player.current());
        if let Some(character) = self.service.player.current() {
            self.args.buffs.iter_mut().for_each(|state| {
                state.update_enabled_state(character, &self.service.settings.current());
            });
        }
        self.service.rotator.update(
            self.args.rotator,
            self.service.minimap.current(),
            self.service.player.current(),
            &self.service.settings.current(),
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
