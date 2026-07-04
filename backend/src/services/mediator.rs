use std::{fmt::Debug, ops::DerefMut};

#[cfg(debug_assertions)]
use std::path::PathBuf;

use opencv::{
    core::{MatTraitConst, MatTraitConstManual, Rect, Vec4b, Vector},
    imgcodecs::{IMREAD_COLOR, IMREAD_GRAYSCALE, imdecode},
};
use tokio::{
    spawn,
    sync::{
        broadcast::{self},
        mpsc, oneshot,
    },
    task::spawn_blocking,
};

use crate::{
    BoundQuadrant, Character, DetectionTemplate, KeyBinding, Operation, OperationUpdate, Request,
    Response, State,
    detect::to_base64_from_mat,
    ecs::{Resources, World},
    minimap::Minimap,
    models::Map,
    operation::OperationState,
    player::Quadrant,
    recv_request,
    services::{Event, EventContext, EventHandler},
    skill::SkillKind,
};
#[cfg(debug_assertions)]
use crate::{DebugState, TransparentShapeDifficulty};

#[derive(Debug)]
pub enum MediatorEvent {
    Ui {
        request: Request,
        response: oneshot::Sender<Response>,
    },
}

impl Event for MediatorEvent {}

/// A service to handle mediation-related incoming requests.
pub trait MediatorService: Debug {
    fn subscribe_state(&self) -> broadcast::Receiver<State>;

    fn broadcast_state(&self, resources: &mut Resources, world: &World);
}

#[derive(Debug)]
pub struct DefaultMediatorService {
    state_tx: broadcast::Sender<State>,
}

impl DefaultMediatorService {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<MediatorEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        spawn(async move {
            loop {
                if let Some((request, response)) = recv_request().await {
                    let _ = event_tx.send(MediatorEvent::Ui { request, response });
                }
            }
        });

        let service = Self {
            state_tx: broadcast::channel(1).0,
        };
        (service, event_rx)
    }
}

impl MediatorService for DefaultMediatorService {
    fn subscribe_state(&self) -> broadcast::Receiver<State> {
        self.state_tx.subscribe()
    }

    fn broadcast_state(&self, resources: &mut Resources, world: &World) {
        if !self.state_tx.is_empty() {
            return;
        }

        let player_context = &world.player.context;
        let state = world.player.state.to_string();
        let health = player_context.health();
        let normal_action = player_context.normal_action_name();
        let priority_action = player_context.priority_action_name();
        let position = player_context.last_known_pos.map(|pos| (pos.x, pos.y));
        let destinations = player_context
            .last_destinations
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|pos| (pos.x, pos.y))
            .collect();

        let input_state = resources.input.state();
        let erda_shower_state = world.skills[SkillKind::ErdaShower].state.to_string();

        let idle = match world.minimap.state {
            Minimap::Idle(idle) => Some(idle),
            Minimap::Detecting => None,
        };

        let portals = idle
            .map(|idle| idle.portals().into_iter().map(Into::into).collect())
            .unwrap_or_default();

        let operation = match resources.operation.state {
            OperationState::TemporaryHalting { resume, .. } => Operation::TemporaryHalting(resume),
            OperationState::Halting => Operation::Halting,
            OperationState::Running => Operation::Running,
            OperationState::RunUntil { instant, .. } => Operation::RunUntil(instant),
        };

        let auto_mob_quadrant =
            player_context
                .auto_mob_last_quadrant()
                .map(|quadrant| match quadrant {
                    Quadrant::TopLeft => BoundQuadrant::TopLeft,
                    Quadrant::TopRight => BoundQuadrant::TopRight,
                    Quadrant::BottomRight => BoundQuadrant::BottomRight,
                    Quadrant::BottomLeft => BoundQuadrant::BottomLeft,
                });
        let detector = resources
            .detector
            .as_ref()
            .map(|_| resources.detector_cloned());

        let sender = self.state_tx.clone();

        spawn_blocking(move || {
            let frame = detector
                .zip(idle)
                .map(|(detector, idle)| minimap_frame_from(idle.bbox, &detector.mat()));

            let state = State {
                position,
                health,
                state,
                normal_action,
                priority_action,
                erda_shower_state,
                input_state,
                destinations,
                operation,
                frame,
                portals,
                auto_mob_quadrant,
            };

            let _ = sender.send(state);
        });
    }
}

pub struct MediatorEventHandler;

impl EventHandler<MediatorEvent> for MediatorEventHandler {
    fn handle(&mut self, context: &mut EventContext<'_>, event: MediatorEvent) {
        match event {
            MediatorEvent::Ui { request, response } => {
                handle_ui_request(context, request, response)
            }
        }
    }
}

fn handle_ui_request(
    context: &mut EventContext<'_>,
    request: Request,
    response: oneshot::Sender<Response>,
) {
    let result = match request {
        Request::UpdateOperation(update) => {
            update_operation(context, update);
            Response::UpdateOperation
        }
        Request::CreateMap(name) => Response::CreateMap(create_map(context, name)),
        Request::UpdateMap(map, preset) => {
            update_map(context, map, preset);
            Response::UpdateMap
        }
        Request::UpdateCharacter(character) => {
            update_character(context, character);
            Response::UpdateCharacter
        }
        Request::RedetectMinimap => {
            redetect_map_minimap(context);
            Response::RedetectMinimap
        }
        Request::StateReceiver => Response::StateReceiver(subscribe_game_state(context)),
        Request::KeyReceiver => Response::KeyReceiver(subscribe_key(context)),
        Request::RefreshCaptureHandles => {
            refresh_capture_handles(context);
            Response::RefreshCaptureHandles
        }
        Request::QueryCaptureHandles => {
            Response::QueryCaptureHandles(query_capture_handles(context))
        }
        Request::SelectCaptureHandle(index) => {
            select_capture_handle(context, index);
            Response::SelectCaptureHandle
        }
        Request::QueryTemplate(template) => {
            Response::QueryTemplate(query_template(context, template))
        }
        Request::ConvertImageToBase64(image, is_grayscale) => {
            Response::ConvertImageToBase64(convert_image_to_base64(image, is_grayscale))
        }
        Request::SaveCaptureImage(is_grayscale) => {
            save_capture_image(context, is_grayscale);
            Response::SaveCaptureImage
        }
        #[cfg(debug_assertions)]
        Request::DebugStateReceiver => Response::DebugStateReceiver(subscribe_debug_state(context)),
        #[cfg(debug_assertions)]
        Request::AutoSaveRune(auto_save) => {
            context.resources.debug.auto_save_rune = auto_save;
            Response::AutoSaveRune
        }
        #[cfg(debug_assertions)]
        Request::AutoRecordLieDetector(auto_record) => {
            context.resources.debug.auto_record_lie_detector = auto_record;
            Response::AutoRecordLieDetector
        }
        #[cfg(debug_assertions)]
        Request::RecordVideo(start) => {
            record_video(context, start);
            Response::RecordVideo
        }
        #[cfg(debug_assertions)]
        Request::TestSpinRune => {
            test_spin_rune(context);
            Response::TestSpinRune
        }
        #[cfg(debug_assertions)]
        Request::TestVioletta => {
            context
                .debug_service
                .test_violetta(context.resources.input.clone());
            Response::TestVioletta
        }
        #[cfg(debug_assertions)]
        Request::TestTransparentShape(difficulty) => {
            test_transparent_shape(context, difficulty);
            Response::TestTransparentShape
        }
        #[cfg(debug_assertions)]
        Request::TestTransparentShapeFile(path) => {
            log::info!("[mediator] TestTransparentShapeFile received: {:?}", path);
            test_transparent_shape_file(context, path);
            log::info!("[mediator] TestTransparentShapeFile handler done");
            Response::TestTransparentShapeFile
        }
    };
    let _ = response.send(result);
}

fn update_operation(context: &mut EventContext<'_>, update: OperationUpdate) {
    if context.map_service.map().is_none() || context.character_service.character().is_none() {
        return;
    }
    context.operation_service.update(context.resources, update);
}

fn create_map(context: &mut EventContext<'_>, name: String) -> Option<Map> {
    context
        .map_service
        .create(context.world.minimap.state, name)
}

fn update_map(context: &mut EventContext<'_>, map: Option<Map>, preset: Option<String>) {
    let world = &mut context.world;
    let map_service = &mut context.map_service;
    map_service.update_map_preset(map, preset);
    map_service.apply(&mut world.minimap.context, &mut world.player.context);

    let rotator_service = &mut context.rotator_service;
    let map = map_service.map();
    let preset = map_service.preset();
    rotator_service.update_from_map(map, preset);
    rotator_service.apply(context.rotator);
}

fn redetect_map_minimap(context: &mut EventContext<'_>) {
    context.map_service.redetect(&mut context.world.minimap);
}

fn update_character(context: &mut EventContext<'_>, character: Option<Character>) {
    let character_service = &mut context.character_service;
    character_service.update_character(character);
    character_service.apply_character(&mut context.world.player.context);

    let character = character_service.character();
    let settings = context.settings_service.settings();
    let rotator_service = &mut context.rotator_service;
    rotator_service.update_from_characters(character);
    if let Some(character) = character {
        context.world.buffs.iter_mut().for_each(|buff| {
            buff.context.update_enabled_state(character, &settings);
        });
    }
    rotator_service.apply(context.rotator);
}

fn subscribe_game_state(context: &mut EventContext<'_>) -> broadcast::Receiver<State> {
    context.mediator_service.subscribe_state()
}

fn subscribe_key(context: &mut EventContext<'_>) -> broadcast::Receiver<KeyBinding> {
    context.input_service.subscribe_key()
}

fn refresh_capture_handles(context: &mut EventContext<'_>) {
    context.capture_service.update_windows();
    select_capture_handle(context, None);
}

fn query_capture_handles(context: &mut EventContext<'_>) -> (Vec<String>, Option<usize>) {
    (
        context.capture_service.window_names(),
        context.capture_service.selected_window_index(),
    )
}

fn select_capture_handle(context: &mut EventContext<'_>, index: Option<usize>) {
    let capture_service = &mut context.capture_service;
    capture_service.update_selected_window(index);
    capture_service.apply_selected_window(context.capture);

    context.input_service.apply_window(
        context.resources.input.deref_mut(),
        capture_service.selected_window(),
    );
}

fn query_template(context: &mut EventContext<'_>, template: DetectionTemplate) -> String {
    context.localization_service.template(template)
}

fn convert_image_to_base64(image: Vec<u8>, is_grayscale: bool) -> Option<String> {
    let flag = if is_grayscale {
        IMREAD_GRAYSCALE
    } else {
        IMREAD_COLOR
    };
    let vector = Vector::<u8>::from_iter(image);
    let mat = imdecode(&vector, flag).ok()?;

    to_base64_from_mat(&mat).ok()
}

fn save_capture_image(context: &mut EventContext<'_>, is_grayscale: bool) {
    context
        .localization_service
        .save_capture_image(context.resources, is_grayscale);
}

#[cfg(debug_assertions)]
fn subscribe_debug_state(context: &mut EventContext<'_>) -> broadcast::Receiver<DebugState> {
    context.debug_service.subscribe_state()
}

#[cfg(debug_assertions)]
fn record_video(context: &mut EventContext<'_>, start: bool) {
    context.debug_service.record_video(context.resources, start);
}

#[cfg(debug_assertions)]
fn test_spin_rune(context: &mut EventContext<'_>) {
    context.debug_service.test_spin_rune();
}

#[cfg(debug_assertions)]
fn test_transparent_shape(context: &mut EventContext<'_>, difficulty: TransparentShapeDifficulty) {
    context
        .debug_service
        .test_transparent_shape(context.resources.input.clone(), difficulty);
}

#[cfg(debug_assertions)]
fn test_transparent_shape_file(context: &mut EventContext<'_>, path: PathBuf) {
    context
        .debug_service
        .test_transparent_shape_file(context.resources.input.clone(), path);
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
