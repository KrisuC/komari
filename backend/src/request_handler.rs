use std::time::Instant;
use std::{sync::LazyLock, time::Duration};

use base64::{Engine, prelude::BASE64_STANDARD};
#[cfg(debug_assertions)]
use include_dir::{Dir, include_dir};
use log::debug;
use opencv::core::Vector;
#[cfg(debug_assertions)]
use opencv::{
    core::{Mat, ModifyInplace},
    imgcodecs::{IMREAD_COLOR, imdecode},
    imgproc::{COLOR_BGR2BGRA, cvt_color_def},
};
use opencv::{
    core::{MatTraitConst, MatTraitConstManual, Rect, Vec4b},
    imgcodecs::imencode_def,
};
use platforms::windows::{Handle, KeyInputKind, KeyKind, KeyReceiver, query_capture_handles};
#[cfg(debug_assertions)]
use rand::distr::{Alphanumeric, SampleString};
use strum::IntoEnumIterator;
use tokio::sync::broadcast;

use crate::DatabaseEvent;
#[cfg(debug_assertions)]
use crate::debug::{
    save_image_for_training, save_image_for_training_to, save_minimap_for_training,
};
#[cfg(debug_assertions)]
use crate::detect::{ArrowsCalibrating, ArrowsState, CachedDetector, Detector};
#[cfg(debug_assertions)]
use crate::mat::OwnedMat;
use crate::pathing::Platform;
use crate::{
    Action, ActionCondition, ActionConfigurationCondition, ActionKey, BoundQuadrant, CaptureMode,
    Character, GameOperation, GameState, KeyBinding, KeyBindingConfiguration,
    Minimap as MinimapData, NavigationPath, PotionMode, RequestHandler, RotationMode, RotatorMode,
    Settings,
    bridge::{ImageCapture, ImageCaptureKind, KeySenderMethod},
    buff::{BuffKind, BuffState},
    context::{Context, Operation},
    database::InputMethod,
    minimap::{Minimap, MinimapState},
    navigation::Navigator,
    player::{PlayerState, Quadrant},
    poll_request,
    rotator::{Rotator, RotatorBuildArgs},
    skill::SkillKind,
};

static GAME_STATE: LazyLock<broadcast::Sender<GameState>> =
    LazyLock::new(|| broadcast::channel(1).0);

// TODO: Add unit tests
pub struct DefaultRequestHandler<'a> {
    pub context: &'a mut Context,
    pub character: &'a mut Option<Character>,
    pub settings: &'a mut Settings,
    pub buffs: &'a mut Vec<(BuffKind, KeyBinding)>,
    pub buff_states: &'a mut Vec<BuffState>,
    pub actions: &'a mut Vec<Action>,
    pub rotator: &'a mut Rotator,
    pub navigator: &'a mut Navigator,
    pub player: &'a mut PlayerState,
    pub minimap: &'a mut MinimapState,
    pub minimap_data: &'a mut Option<MinimapData>,
    pub minimap_data_preset: &'a mut Option<String>,
    pub key_sender: &'a broadcast::Sender<KeyBinding>,
    pub key_receiver: &'a mut KeyReceiver,
    pub image_capture: &'a mut ImageCapture,
    pub capture_handles: &'a mut Vec<(String, Handle)>,
    pub selected_capture_handle: &'a mut Option<Handle>,
    pub database_event_receiver: &'a mut broadcast::Receiver<DatabaseEvent>,
    #[cfg(debug_assertions)]
    pub recording_images_id: &'a mut Option<String>,
    #[cfg(debug_assertions)]
    pub infering_rune: &'a mut Option<(ArrowsCalibrating, Instant)>,
}

impl DefaultRequestHandler<'_> {
    pub fn poll_request(&mut self) {
        poll_request(self);
        poll_key(self);
        poll_database_event(self);
        #[cfg(debug_assertions)]
        poll_debug(self);

        if GAME_STATE.is_empty() {
            // TODO: Separate into variables for better readability
            let game_state = GameState {
                position: self.player.last_known_pos.map(|pos| (pos.x, pos.y)),
                health: self.player.health(),
                state: self.context.player.to_string(),
                normal_action: self.player.normal_action_name(),
                priority_action: self.player.priority_action_name(),
                erda_shower_state: self.context.skills[SkillKind::ErdaShower].to_string(),
                destinations: self
                    .player
                    .last_destinations
                    .clone()
                    .map(|points| {
                        points
                            .into_iter()
                            .map(|point| (point.x, point.y))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                operation: match self.context.operation {
                    Operation::HaltUntil(instant) => GameOperation::HaltUntil(instant),
                    Operation::Halting => GameOperation::Halting,
                    Operation::Running => GameOperation::Running,
                    Operation::RunUntil(instant) => GameOperation::RunUntil(instant),
                },
                frame: self
                    .context
                    .detector
                    .as_ref()
                    .map(|detector| detector.mat())
                    .and_then(|mat| extract_minimap(self.context, mat)),
                platforms_bound: if self
                    .minimap_data
                    .as_ref()
                    .is_some_and(|data| data.auto_mob_platforms_bound)
                    && let Minimap::Idle(idle) = self.context.minimap
                {
                    idle.platforms_bound.map(|bound| bound.into())
                } else {
                    None
                },
                portals: if let Minimap::Idle(idle) = self.context.minimap {
                    idle.portals()
                        .into_iter()
                        .map(|portal| portal.into())
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                },
                auto_mob_quadrant: self.player.auto_mob_last_quadrant().map(|quadrant| {
                    match quadrant {
                        Quadrant::TopLeft => BoundQuadrant::TopLeft,
                        Quadrant::TopRight => BoundQuadrant::TopRight,
                        Quadrant::BottomRight => BoundQuadrant::BottomRight,
                        Quadrant::BottomLeft => BoundQuadrant::BottomLeft,
                    }
                }),
            };
            let _ = GAME_STATE.send(game_state);
        }
    }

    fn update_rotator_actions(&mut self) {
        let mode = self
            .minimap_data
            .as_ref()
            .map(|minimap| match minimap.rotation_mode {
                RotationMode::StartToEnd => RotatorMode::StartToEnd,
                RotationMode::StartToEndThenReverse => RotatorMode::StartToEndThenReverse,
                RotationMode::AutoMobbing => RotatorMode::AutoMobbing(
                    minimap.rotation_mobbing_key,
                    minimap.rotation_auto_mob_bound,
                ),
                RotationMode::PingPong => RotatorMode::PingPong(
                    minimap.rotation_mobbing_key,
                    minimap.rotation_ping_pong_bound,
                ),
            })
            .unwrap_or_default();
        let reset_on_erda = self
            .minimap_data
            .as_ref()
            .map(|minimap| minimap.actions_any_reset_on_erda_condition)
            .unwrap_or_default();
        let actions = self
            .character
            .as_ref()
            .map(|character| {
                config_actions(character)
                    .into_iter()
                    .chain(self.actions.iter().copied())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let args = RotatorBuildArgs {
            mode,
            actions: actions.as_slice(),
            buffs: self.buffs,
            familiar_essence_key: self
                .character
                .as_ref()
                .map(|character| character.familiar_essence_key.key)
                .unwrap_or_default(),
            familiar_swappable_slots: self.settings.familiars.swappable_familiars,
            familiar_swappable_rarities: &self.settings.familiars.swappable_rarities,
            familiar_swap_check_millis: self.settings.familiars.swap_check_millis,
            elite_boss_behavior: self.character.as_ref().and_then(|character| {
                character
                    .elite_boss_behavior_enabled
                    .then_some(character.elite_boss_behavior)
            }),
            elite_boss_behavior_key: self
                .character
                .as_ref()
                .map(|character| character.elite_boss_behavior_key)
                .unwrap_or_default(),
            enable_panic_mode: self.settings.enable_panic_mode,
            enable_rune_solving: self.settings.enable_rune_solving,
            enable_familiars_swapping: self.settings.familiars.enable_familiars_swapping,
            enable_reset_normal_actions_on_erda: reset_on_erda,
        };

        self.rotator.build_actions(args);
    }

    pub fn update_context_halting(&mut self, halting: bool, reset_player_to_idle: bool) {
        if self.minimap_data.as_ref().is_some() && self.character.is_some() {
            self.context.operation = match (halting, self.settings.cycle_run_stop) {
                (true, _) => Operation::Halting,
                (false, true) => Instant::now()
                    .checked_add(Duration::from_millis(
                        self.settings.cycle_run_duration_millis,
                    ))
                    .map(Operation::RunUntil)
                    .unwrap_or(Operation::Running),
                (false, false) => Operation::Running,
            };
            if halting {
                self.rotator.reset_queue();
                self.player.clear_actions_aborted(reset_player_to_idle);
            }
        }
    }

    fn update_settings(&mut self, settings: Settings) {
        let mut handle_or_default = self.selected_capture_handle.unwrap_or(self.context.handle);

        if settings.capture_mode != self.settings.capture_mode {
            self.image_capture
                .set_mode(handle_or_default, settings.capture_mode);
        }

        if settings.input_method != self.settings.input_method
            || settings.input_method_rpc_server_url != self.settings.input_method_rpc_server_url
        {
            if let ImageCaptureKind::BitBltArea(capture) = self.image_capture.kind() {
                handle_or_default = capture.handle();
                *self.key_receiver = KeyReceiver::new(handle_or_default, KeyInputKind::Foreground);
            }
            match settings.input_method {
                InputMethod::Default => {
                    let kind = if matches!(settings.capture_mode, CaptureMode::BitBltArea) {
                        KeyInputKind::Foreground
                    } else {
                        KeyInputKind::Fixed
                    };
                    self.context
                        .keys
                        .set_method(KeySenderMethod::Default(handle_or_default, kind));
                }
                InputMethod::Rpc => {
                    self.context.keys.set_method(KeySenderMethod::Rpc(
                        handle_or_default,
                        settings.input_method_rpc_server_url.clone(),
                    ));
                }
            }
        }
        self.context.operation = match self.context.operation {
            Operation::HaltUntil(_) => {
                if settings.cycle_run_stop {
                    Operation::HaltUntil(
                        Instant::now() + Duration::from_millis(settings.cycle_stop_duration_millis),
                    )
                } else {
                    Operation::Halting
                }
            }
            Operation::Halting => Operation::Halting,
            Operation::Running | Operation::RunUntil(_) => {
                if settings.cycle_run_stop {
                    Operation::RunUntil(
                        Instant::now() + Duration::from_millis(settings.cycle_run_duration_millis),
                    )
                } else {
                    Operation::Running
                }
            }
        };
        *self.settings = settings;

        let Some(character) = self.character else {
            return;
        };
        self.buff_states.iter_mut().for_each(|state| {
            state.update_enabled_state(character, self.settings);
        });
        self.update_rotator_actions();
    }
}

impl RequestHandler for DefaultRequestHandler<'_> {
    fn on_rotate_actions(&mut self, halting: bool) {
        self.update_context_halting(halting, true);
    }

    fn on_create_minimap(&self, name: String) -> Option<MinimapData> {
        if let Minimap::Idle(idle) = self.context.minimap {
            Some(MinimapData {
                name,
                width: idle.bbox.width,
                height: idle.bbox.height,
                ..MinimapData::default()
            })
        } else {
            None
        }
    }

    fn on_update_minimap(&mut self, preset: Option<String>, minimap: Option<MinimapData>) {
        *self.minimap_data = minimap;
        *self.minimap_data_preset = preset.clone();
        self.minimap.set_platforms(
            self.minimap_data
                .as_ref()
                .map(|data| {
                    data.platforms
                        .iter()
                        .copied()
                        .map(Platform::from)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        );
        self.player.reset();

        let Some(minimap) = self.minimap_data.as_ref() else {
            *self.actions = Vec::new();
            self.update_rotator_actions();
            return;
        };

        self.player.config.rune_platforms_pathing = minimap.rune_platforms_pathing;
        self.player.config.rune_platforms_pathing_up_jump_only =
            minimap.rune_platforms_pathing_up_jump_only;
        self.player.config.auto_mob_platforms_pathing = minimap.auto_mob_platforms_pathing;
        self.player.config.auto_mob_platforms_pathing_up_jump_only =
            minimap.auto_mob_platforms_pathing_up_jump_only;
        self.player.config.auto_mob_platforms_bound = minimap.auto_mob_platforms_bound;
        *self.actions = preset
            .and_then(|preset| minimap.actions.get(&preset).cloned())
            .unwrap_or_default();
        self.navigator.mark_dirty_with_destination(minimap.path_id);
        self.update_rotator_actions();
    }

    fn on_create_navigation_path(&self) -> Option<NavigationPath> {
        if let Some((minimap_base64, name_base64, name_bbox)) =
            extract_minimap_and_name_base64(self.context)
        {
            Some(NavigationPath {
                id: None,
                minimap_snapshot_base64: minimap_base64,
                name_snapshot_base64: name_base64,
                name_snapshot_width: name_bbox.width,
                name_snapshot_height: name_bbox.height,
                points: vec![],
            })
        } else {
            None
        }
    }

    fn on_recapture_navigation_path(&self, mut path: NavigationPath) -> NavigationPath {
        if let Some((minimap_base64, name_base64, name_bbox)) =
            extract_minimap_and_name_base64(self.context)
        {
            path.minimap_snapshot_base64 = minimap_base64;
            path.name_snapshot_base64 = name_base64;
            path.name_snapshot_width = name_bbox.width;
            path.name_snapshot_height = name_bbox.height;
        }

        path
    }

    fn on_update_character(&mut self, character: Option<Character>) {
        *self.character = character;

        let Some(character) = self.character else {
            return;
        };
        *self.buffs = config_buffs(character);
        self.player.reset();
        self.player.config.class = character.class;
        self.player.config.disable_adjusting = character.disable_adjusting;
        self.player.config.interact_key = character.interact_key.key.into();
        self.player.config.grappling_key = character.ropelift_key.map(|key| key.key.into());
        self.player.config.teleport_key = character.teleport_key.map(|key| key.key.into());
        self.player.config.jump_key = character.jump_key.key.into();
        self.player.config.upjump_key = character.up_jump_key.map(|key| key.key.into());
        self.player.config.cash_shop_key = character.cash_shop_key.key.into();
        self.player.config.familiar_key = character.familiar_menu_key.key.into();
        self.player.config.to_town_key = character.to_town_key.key.into();
        self.player.config.change_channel_key = character.change_channel_key.key.into();
        self.player.config.potion_key = character.potion_key.key.into();
        self.player.config.use_potion_below_percent =
            match (character.potion_key.enabled, character.potion_mode) {
                (false, _) | (_, PotionMode::EveryMillis(_)) => None,
                (_, PotionMode::Percentage(percent)) => Some(percent / 100.0),
            };
        self.player.config.update_health_millis = Some(character.health_update_millis);
        self.buff_states.iter_mut().for_each(|state| {
            state.update_enabled_state(character, self.settings);
        });
        self.update_rotator_actions();
    }

    #[inline]
    fn on_redetect_minimap(&mut self) {
        self.context.minimap = Minimap::Detecting;
    }

    #[inline]
    fn on_game_state_receiver(&self) -> broadcast::Receiver<GameState> {
        GAME_STATE.subscribe()
    }

    #[inline]
    fn on_key_receiver(&self) -> broadcast::Receiver<KeyBinding> {
        self.key_sender.subscribe()
    }

    fn on_query_capture_handles(&mut self) -> (Vec<String>, Option<usize>) {
        *self.capture_handles = query_capture_handles();

        let names = self
            .capture_handles
            .iter()
            .map(|(name, _)| name)
            .cloned()
            .collect::<Vec<_>>();
        let selected = if let Some(selected_handle) = self.selected_capture_handle {
            self.capture_handles
                .iter()
                .enumerate()
                .find(|(_, (_, handle))| handle == selected_handle)
                .map(|(i, _)| i)
        } else {
            None
        };
        (names, selected)
    }

    fn on_select_capture_handle(&mut self, index: Option<usize>) {
        if matches!(self.settings.capture_mode, CaptureMode::BitBltArea) {
            return;
        }

        let handle = index
            .and_then(|index| self.capture_handles.get(index))
            .map(|(_, handle)| *handle);
        let handle_or_default = handle.unwrap_or(self.context.handle);

        *self.selected_capture_handle = handle;
        self.image_capture
            .set_mode(handle_or_default, self.settings.capture_mode);
        *self.key_receiver = KeyReceiver::new(handle_or_default, KeyInputKind::Fixed);
        match self.settings.input_method {
            InputMethod::Default => {
                self.context.keys.set_method(KeySenderMethod::Default(
                    handle_or_default,
                    KeyInputKind::Fixed,
                ));
            }
            InputMethod::Rpc => {
                self.context.keys.set_method(KeySenderMethod::Rpc(
                    handle_or_default,
                    self.settings.input_method_rpc_server_url.clone(),
                ));
            }
        }
    }

    #[cfg(debug_assertions)]
    fn on_capture_image(&self, is_grayscale: bool) {
        if let Some(ref detector) = self.context.detector {
            save_image_for_training(detector.mat(), is_grayscale, false);
        }
    }

    #[cfg(debug_assertions)]
    fn on_infer_rune(&mut self) {
        *self.infering_rune = Some((ArrowsCalibrating::default(), Instant::now()));
    }

    #[cfg(debug_assertions)]
    fn on_infer_minimap(&self) {
        if let Some(ref detector) = self.context.detector {
            // FIXME: 160 matches one in minimap.rs
            if let Ok(rect) = detector.detect_minimap(160) {
                save_minimap_for_training(detector.mat(), rect);
            }
        }
    }

    #[cfg(debug_assertions)]
    fn on_record_images(&mut self, start: bool) {
        *self.recording_images_id = if start {
            Some(Alphanumeric.sample_string(&mut rand::rng(), 8))
        } else {
            None
        };
    }

    #[cfg(debug_assertions)]
    fn on_test_spin_rune(&self) {
        static SPIN_TEST_DIR: Dir<'static> = include_dir!("$SPIN_TEST_DIR");
        static SPIN_TEST_IMAGES: LazyLock<Vec<Mat>> = LazyLock::new(|| {
            let mut files = SPIN_TEST_DIR.files().collect::<Vec<_>>();
            files.sort_by_key(|file| file.path().to_str().unwrap());
            files
                .into_iter()
                .map(|file| {
                    let vec = Vector::from_slice(file.contents());
                    let mut mat = imdecode(&vec, IMREAD_COLOR).unwrap();
                    unsafe {
                        mat.modify_inplace(|mat, mat_mut| {
                            cvt_color_def(mat, mat_mut, COLOR_BGR2BGRA).unwrap();
                        });
                    }
                    mat
                })
                .collect()
        });

        let mut calibrating = ArrowsCalibrating::default();
        calibrating.enable_spin_test();

        for mat in &*SPIN_TEST_IMAGES {
            match CachedDetector::new(OwnedMat::from(mat.clone())).detect_rune_arrows(calibrating) {
                Ok(ArrowsState::Complete(arrows)) => {
                    debug!(target: "test", "spin test completed {arrows:?}");
                }
                Ok(ArrowsState::Calibrating(new_calibrating)) => {
                    calibrating = new_calibrating;
                }
                Err(err) => {
                    debug!(target: "test", "spin test error {err}");
                    break;
                }
            }
        }
    }
}

// TODO: should only handle a single matched key binding
#[inline]
fn poll_key(handler: &mut DefaultRequestHandler) {
    let Some(received_key) = handler.key_receiver.try_recv() else {
        return;
    };
    debug!(target: "handler", "received key {received_key:?}");
    if let KeyBindingConfiguration { key, enabled: true } = handler.settings.toggle_actions_key
        && KeyKind::from(key) == received_key
    {
        handler.on_rotate_actions(!handler.context.operation.halting());
    }
    let _ = handler.key_sender.send(received_key.into());
}

#[inline]
fn poll_database_event(handler: &mut DefaultRequestHandler) {
    let Ok(event) = handler.database_event_receiver.try_recv() else {
        return;
    };
    debug!(target: "handler", "received database event {event:?}");
    match event {
        DatabaseEvent::MinimapUpdated(minimap) => {
            let id = minimap
                .id
                .expect("valid minimap id if updated from database");
            if Some(id) == handler.minimap_data.as_ref().and_then(|minimap| minimap.id) {
                handler.on_update_minimap(handler.minimap_data_preset.clone(), Some(minimap));
            }
        }
        DatabaseEvent::MinimapDeleted(deleted_id) => {
            if Some(deleted_id) == handler.minimap_data.as_ref().and_then(|minimap| minimap.id) {
                handler.on_update_minimap(None, None);
            }
        }
        DatabaseEvent::NavigationPathUpdated | DatabaseEvent::NavigationPathDeleted => {
            handler.navigator.mark_dirty();
        }
        DatabaseEvent::SettingsUpdated(settings) => handler.update_settings(settings),
        DatabaseEvent::CharacterUpdated(character) => {
            let updated_id = character
                .id
                .expect("valid character id if updated from database");
            let current_id = handler
                .character
                .as_ref()
                .and_then(|character| character.id);

            if Some(updated_id) == current_id {
                handler.on_update_character(Some(character));
            }
        }
        DatabaseEvent::CharacterDeleted(deleted_id) => {
            let current_id = handler
                .character
                .as_ref()
                .and_then(|character| character.id);
            if Some(deleted_id) == current_id {
                handler.on_update_character(None);
            }
        }
    }
}

#[cfg(debug_assertions)]
fn poll_debug(handler: &mut DefaultRequestHandler) {
    if let Some((calibrating, instant)) = handler.infering_rune.as_ref().copied() {
        if instant.elapsed().as_secs() >= 10 {
            debug!(target: "debug", "infer rune timed out");
            *handler.infering_rune = None;
        } else {
            match handler
                .context
                .detector_unwrap()
                .detect_rune_arrows(calibrating)
            {
                Ok(ArrowsState::Complete(arrows)) => {
                    debug!(target: "debug", "infer rune result {arrows:?}");
                    // TODO: Save
                    *handler.infering_rune = None;
                }
                Ok(ArrowsState::Calibrating(calibrating)) => {
                    *handler.infering_rune = Some((calibrating, instant));
                }
                Err(err) => {
                    debug!(target: "debug", "infer rune failed {err}");
                    *handler.infering_rune = None;
                }
            }
        }
    }

    if let Some(id) = handler.recording_images_id.clone() {
        save_image_for_training_to(
            handler.context.detector_unwrap().mat(),
            Some(id),
            false,
            false,
        );
    }
}

// TODO: Better way?
fn extract_minimap_and_name_base64(context: &Context) -> Option<(String, String, Rect)> {
    if let Minimap::Idle(idle) = context.minimap
        && let Some(detector) = context.detector.as_ref()
    {
        let name_bbox = detector.detect_minimap_name(idle.bbox).ok()?;
        let name = detector.grayscale_mat().roi(name_bbox).ok()?;
        let mut name_bytes = Vector::new();
        imencode_def(".png", &name, &mut name_bytes).ok()?;
        let name_base64 = BASE64_STANDARD.encode(name_bytes);

        let minimap = detector.mat().roi(idle.bbox).ok()?;
        let mut minimap_bytes = Vector::new();
        imencode_def(".png", &minimap, &mut minimap_bytes).ok()?;
        let minimap_base64 = BASE64_STANDARD.encode(minimap_bytes);

        Some((minimap_base64, name_base64, name_bbox))
    } else {
        None
    }
}

#[inline]
fn extract_minimap(context: &Context, mat: &impl MatTraitConst) -> Option<(Vec<u8>, usize, usize)> {
    if let Minimap::Idle(idle) = context.minimap {
        let minimap = mat
            .roi(idle.bbox)
            .unwrap()
            .iter::<Vec4b>()
            .unwrap()
            .flat_map(|bgra| {
                let bgra = bgra.1;
                [bgra[2], bgra[1], bgra[0], 255]
            })
            .collect::<Vec<u8>>();
        return Some((minimap, idle.bbox.width as usize, idle.bbox.height as usize));
    }
    None
}

fn config_buffs(character: &Character) -> Vec<(BuffKind, KeyBinding)> {
    BuffKind::iter()
        .filter_map(|kind| {
            let enabled_key = match kind {
                BuffKind::Rune => None, // Internal buff
                BuffKind::Familiar => character
                    .familiar_buff_key
                    .enabled
                    .then_some(character.familiar_buff_key.key),
                BuffKind::SayramElixir => character
                    .sayram_elixir_key
                    .enabled
                    .then_some(character.sayram_elixir_key.key),
                BuffKind::AureliaElixir => character
                    .aurelia_elixir_key
                    .enabled
                    .then_some(character.aurelia_elixir_key.key),
                BuffKind::ExpCouponX3 => character
                    .exp_x3_key
                    .enabled
                    .then_some(character.exp_x3_key.key),
                BuffKind::BonusExpCoupon => character
                    .bonus_exp_key
                    .enabled
                    .then_some(character.bonus_exp_key.key),
                BuffKind::LegionLuck => character
                    .legion_luck_key
                    .enabled
                    .then_some(character.legion_luck_key.key),
                BuffKind::LegionWealth => character
                    .legion_wealth_key
                    .enabled
                    .then_some(character.legion_wealth_key.key),
                BuffKind::WealthAcquisitionPotion => character
                    .wealth_acquisition_potion_key
                    .enabled
                    .then_some(character.wealth_acquisition_potion_key.key),
                BuffKind::ExpAccumulationPotion => character
                    .exp_accumulation_potion_key
                    .enabled
                    .then_some(character.exp_accumulation_potion_key.key),
                BuffKind::ExtremeRedPotion => character
                    .extreme_red_potion_key
                    .enabled
                    .then_some(character.extreme_red_potion_key.key),
                BuffKind::ExtremeBluePotion => character
                    .extreme_blue_potion_key
                    .enabled
                    .then_some(character.extreme_blue_potion_key.key),
                BuffKind::ExtremeGreenPotion => character
                    .extreme_green_potion_key
                    .enabled
                    .then_some(character.extreme_green_potion_key.key),
                BuffKind::ExtremeGoldPotion => character
                    .extreme_gold_potion_key
                    .enabled
                    .then_some(character.extreme_gold_potion_key.key),
            };
            Some(kind).zip(enabled_key)
        })
        .collect()
}

fn config_actions(character: &Character) -> Vec<Action> {
    let mut vec = Vec::new();
    if let KeyBindingConfiguration { key, enabled: true } = character.feed_pet_key {
        let feed_pet_action = Action::Key(ActionKey {
            key,
            count: 1,
            condition: ActionCondition::EveryMillis(character.feed_pet_millis),
            wait_before_use_millis: 350,
            wait_after_use_millis: 350,
            ..ActionKey::default()
        });
        for _ in 0..character.num_pets {
            vec.push(feed_pet_action);
        }
    }
    if let KeyBindingConfiguration { key, enabled: true } = character.potion_key
        && let PotionMode::EveryMillis(millis) = character.potion_mode
    {
        vec.push(Action::Key(ActionKey {
            key,
            count: 1,
            condition: ActionCondition::EveryMillis(millis),
            wait_before_use_millis: 350,
            wait_after_use_millis: 350,
            ..ActionKey::default()
        }));
    }

    let mut i = 0;
    let config_actions = &character.actions;
    while i < config_actions.len() {
        let action = config_actions[i];
        let enabled = action.enabled;

        if enabled {
            vec.push(action.into());
        }
        while i + 1 < config_actions.len() {
            let action = config_actions[i + 1];
            if !matches!(action.condition, ActionConfigurationCondition::Linked) {
                break;
            }
            if enabled {
                vec.push(action.into());
            }
            i += 1;
        }

        i += 1;
    }
    vec
}
