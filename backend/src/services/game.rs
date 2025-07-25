use std::time::{Duration, Instant};

use log::debug;
use opencv::core::{MatTraitConst, MatTraitConstManual, Rect, Vec4b};
use platforms::windows::{KeyKind, KeyReceiver};
use strum::IntoEnumIterator;
use tokio::{
    spawn,
    sync::broadcast::{self, Receiver, Sender},
};

use crate::{
    Action, ActionCondition, ActionConfigurationCondition, ActionKey, BoundQuadrant, Character,
    DatabaseEvent, GameOperation, GameState, KeyBinding, KeyBindingConfiguration, Minimap,
    PotionMode, Settings,
    array::Array,
    buff::BuffKind,
    context::{Context, Operation},
    database_event_receiver, minimap,
    player::{PlayerState, Quadrant},
    rotator::Rotator,
    skill::SkillKind,
};

#[derive(Debug)]
pub enum GameEvent {
    ToggleOperation,
    MinimapUpdated(Option<Minimap>),
    CharacterUpdated(Option<Character>),
    SettingsUpdated(Settings),
    NavigationPathsUpdated,
}

/// A service to handle game-related incoming requests and event polling.
#[derive(Debug)]
pub struct GameService {
    key_receiver: KeyReceiver,
    key_sender: Sender<KeyBinding>,
    database_event_receiver: Receiver<DatabaseEvent>,
    game_state_sender: Sender<GameState>,
    game_actions: Vec<Action>,
    game_buffs: Vec<(BuffKind, KeyBinding)>,
}

impl GameService {
    pub fn new(key_receiver: KeyReceiver) -> Self {
        Self {
            key_receiver,
            key_sender: broadcast::channel(1).0,
            database_event_receiver: database_event_receiver(),
            game_state_sender: broadcast::channel(1).0,
            game_actions: vec![],
            game_buffs: vec![],
        }
    }

    pub fn current_actions(&self) -> &[Action] {
        &self.game_actions
    }

    pub fn current_buffs(&self) -> &[(BuffKind, KeyBinding)] {
        &self.game_buffs
    }

    pub fn current_key_receiver_mut(&mut self) -> &mut KeyReceiver {
        &mut self.key_receiver
    }

    pub fn broadcast_state(
        &self,
        context: &Context,
        player: &PlayerState,
        minimap: Option<&Minimap>,
    ) {
        if self.game_state_sender.is_empty() {
            let position = player.last_known_pos.map(|pos| (pos.x, pos.y));
            let state = context.player.to_string();
            let health = player.health();
            let normal_action = player.normal_action_name();
            let priority_action = player.priority_action_name();
            let erda_shower_state = context.skills[SkillKind::ErdaShower].to_string();
            let destinations = player
                .last_destinations
                .clone()
                .map(|points| {
                    points
                        .into_iter()
                        .map(|point| (point.x, point.y))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let operation = match context.operation {
                Operation::HaltUntil(instant) => GameOperation::HaltUntil(instant),
                Operation::Halting => GameOperation::Halting,
                Operation::Running => GameOperation::Running,
                Operation::RunUntil(instant) => GameOperation::RunUntil(instant),
            };
            let idle = if let minimap::Minimap::Idle(idle) = context.minimap {
                Some(idle)
            } else {
                None
            };
            let platforms_bound = if minimap.is_some_and(|data| data.auto_mob_platforms_bound)
                && let Some(idle) = idle
            {
                idle.platforms_bound.map(|bound| bound.into())
            } else {
                None
            };
            let portals = if let Some(idle) = idle {
                idle.portals()
                    .into_iter()
                    .map(|portal| portal.into())
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            let auto_mob_quadrant =
                player
                    .auto_mob_last_quadrant()
                    .map(|quadrant| match quadrant {
                        Quadrant::TopLeft => BoundQuadrant::TopLeft,
                        Quadrant::TopRight => BoundQuadrant::TopRight,
                        Quadrant::BottomRight => BoundQuadrant::BottomRight,
                        Quadrant::BottomLeft => BoundQuadrant::BottomLeft,
                    });
            let detector = if context.detector.is_some() {
                Some(context.detector_cloned_unwrap())
            } else {
                None
            };
            let sender = self.game_state_sender.clone();

            spawn(async move {
                let frame = if let Some((detector, idle)) = detector.zip(idle) {
                    Some(minimap_frame_from(idle.bbox, detector.mat()))
                } else {
                    None
                };
                let game_state = GameState {
                    position,
                    health,
                    state,
                    normal_action,
                    priority_action,
                    erda_shower_state,
                    destinations,
                    operation,
                    frame,
                    platforms_bound,
                    portals,
                    auto_mob_quadrant,
                };
                let _ = sender.send(game_state);
            });
        }
    }

    pub fn subscribe_state(&self) -> Receiver<GameState> {
        self.game_state_sender.subscribe()
    }

    pub fn subscribe_key(&self) -> Receiver<KeyBinding> {
        self.key_sender.subscribe()
    }

    pub fn poll_events(
        &mut self,
        minimap_id: Option<i64>,
        character_id: Option<i64>,
        settings: &Settings,
    ) -> Array<GameEvent, 2> {
        let mut events = Array::new();

        if let Some(event) = poll_key(self, settings) {
            events.push(event);
        }
        if let Some(event) = poll_database(self, minimap_id, character_id) {
            events.push(event);
        }

        events
    }

    pub fn update_operation(
        &self,
        operation: &mut Operation,
        rotator: &mut Rotator,
        player: &mut PlayerState,
        settings: &Settings,
        halting: bool,
    ) {
        *operation = match (halting, settings.cycle_run_stop) {
            (true, _) => Operation::Halting,
            (false, true) => Instant::now()
                .checked_add(Duration::from_millis(settings.cycle_run_duration_millis))
                .map(Operation::RunUntil)
                .unwrap_or(Operation::Running),
            (false, false) => Operation::Running,
        };
        if halting {
            rotator.reset_queue();
            player.clear_actions_aborted(true);
        }
    }

    pub fn update_actions(
        &mut self,
        minimap: Option<&Minimap>,
        preset: Option<String>,
        character: Option<&Character>,
    ) {
        self.game_actions = minimap
            .zip(preset)
            .and_then(|(minimap, preset)| minimap.actions.get(&preset).cloned())
            .zip(character)
            .map(|(actions, character)| [actions_from(character), actions].concat())
            .unwrap_or_default();
    }

    pub fn update_buffs(&mut self, character: Option<&Character>) {
        self.game_buffs = character.map(buffs_from).unwrap_or_default();
    }
}

#[inline]
fn minimap_frame_from(bbox: Rect, mat: &impl MatTraitConst) -> (Vec<u8>, usize, usize) {
    let minimap = mat
        .roi(bbox)
        .unwrap()
        .iter::<Vec4b>()
        .unwrap()
        .flat_map(|bgra| {
            let bgra = bgra.1;
            [bgra[2], bgra[1], bgra[0], 255]
        })
        .collect::<Vec<u8>>();
    (minimap, bbox.width as usize, bbox.height as usize)
}

fn buffs_from(character: &Character) -> Vec<(BuffKind, KeyBinding)> {
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

fn actions_from(character: &Character) -> Vec<Action> {
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

// TODO: should only handle a single matched key binding
#[inline]
fn poll_key(service: &mut GameService, settings: &Settings) -> Option<GameEvent> {
    let received_key = service.key_receiver.try_recv()?;
    debug!(target: "event", "received key {received_key:?}");

    if let KeyBindingConfiguration { key, enabled: true } = settings.toggle_actions_key
        && KeyKind::from(key) == received_key
    {
        return Some(GameEvent::ToggleOperation);
    }

    let _ = service.key_sender.send(received_key.into());
    None
}

#[inline]
fn poll_database(
    service: &mut GameService,
    minimap_id: Option<i64>,
    character_id: Option<i64>,
) -> Option<GameEvent> {
    let event = service.database_event_receiver.try_recv().ok()?;
    debug!(target: "handler", "received database event {event:?}");

    match event {
        DatabaseEvent::MinimapUpdated(minimap) => {
            let id = minimap
                .id
                .expect("valid minimap id if updated from database");
            if Some(id) == minimap_id {
                return Some(GameEvent::MinimapUpdated(Some(minimap)));
            }
        }
        DatabaseEvent::MinimapDeleted(deleted_id) => {
            if Some(deleted_id) == minimap_id {
                return Some(GameEvent::MinimapUpdated(None));
            }
        }
        DatabaseEvent::NavigationPathsUpdated | DatabaseEvent::NavigationPathsDeleted => {
            return Some(GameEvent::NavigationPathsUpdated);
        }
        DatabaseEvent::SettingsUpdated(settings) => {
            return Some(GameEvent::SettingsUpdated(settings));
        }
        DatabaseEvent::CharacterUpdated(character) => {
            let updated_id = character
                .id
                .expect("valid character id if updated from database");
            if Some(updated_id) == character_id {
                return Some(GameEvent::CharacterUpdated(Some(character)));
            }
        }
        DatabaseEvent::CharacterDeleted(deleted_id) => {
            if Some(deleted_id) == character_id {
                return Some(GameEvent::CharacterUpdated(None));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {}
