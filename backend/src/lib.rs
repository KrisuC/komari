#![feature(new_range_api)]
#![feature(slice_pattern)]
#![feature(map_try_insert)]
#![feature(variant_count)]
#![feature(iter_array_chunks)]
#![feature(associated_type_defaults)]
#![feature(assert_matches)]

use std::{
    sync::{LazyLock, Mutex},
    time::Instant,
};

use strum::Display;
use tokio::{
    sync::{
        broadcast, mpsc,
        oneshot::{self, Sender},
    },
    task::spawn_blocking,
};

mod array;
mod bridge;
mod buff;
mod context;
mod database;
#[cfg(debug_assertions)]
mod debug;
mod detect;
mod mat;
mod minimap;
mod navigation;
mod network;
mod pathing;
mod player;
mod request_handler;
mod rng;
mod rotator;
mod rpc;
mod skill;
mod task;

pub use {
    context::init,
    database::{
        Action, ActionCondition, ActionConfiguration, ActionConfigurationCondition, ActionKey,
        ActionKeyDirection, ActionKeyWith, ActionMove, Bound, CaptureMode, Character, Class,
        DatabaseEvent, EliteBossBehavior, FamiliarRarity, Familiars, InputMethod, KeyBinding,
        KeyBindingConfiguration, LinkKeyBinding, Minimap, MobbingKey, NavigationPath,
        NavigationPoint, NavigationTransition, Notifications, Platform, Position, PotionMode,
        RotationMode, Settings, SwappableFamiliars, database_event_receiver,
    },
    pathing::MAX_PLATFORMS_COUNT,
    rotator::RotatorMode,
    strum::{EnumMessage, IntoEnumIterator, ParseError},
};

type RequestItem = (Request, Sender<Response>);

static REQUESTS: LazyLock<(
    mpsc::Sender<RequestItem>,
    Mutex<mpsc::Receiver<RequestItem>>,
)> = LazyLock::new(|| {
    let (tx, rx) = mpsc::channel::<RequestItem>(10);
    (tx, Mutex::new(rx))
});

macro_rules! expect_unit_variant {
    ($e:expr, $p:path) => {
        match $e {
            $p => (),
            _ => unreachable!(),
        }
    };
}

macro_rules! expect_value_variant {
    ($e:expr, $p:path) => {
        match $e {
            $p(value) => value,
            _ => unreachable!(),
        }
    };
}

/// Represents request from UI.
#[derive(Debug)]
enum Request {
    RotateActions(bool),
    CreateMinimap(String),
    UpdateMinimap(Option<String>, Option<Minimap>),
    CreateNavigationPath,
    RecaptureNavigationPath(NavigationPath),
    UpdateCharacter(Option<Character>),
    RedetectMinimap,
    GameStateReceiver,
    KeyReceiver,
    QueryCaptureHandles,
    SelectCaptureHandle(Option<usize>),
    #[cfg(debug_assertions)]
    CaptureImage(bool),
    #[cfg(debug_assertions)]
    InferRune,
    #[cfg(debug_assertions)]
    InferMinimap,
    #[cfg(debug_assertions)]
    RecordImages(bool),
    #[cfg(debug_assertions)]
    TestSpinRune,
}

/// Represents response to UI [`Request`].
///
/// All internal (e.g. OpenCV) structs must be converted to either database structs
/// or appropriate counterparts before passing to UI.
#[derive(Debug)]
enum Response {
    RotateActions,
    CreateMinimap(Option<Minimap>),
    UpdateMinimap,
    CreateNavigationPath(Option<NavigationPath>),
    RecaptureNavigationPath(NavigationPath),
    UpdateCharacter,
    RedetectMinimap,
    GameStateReceiver(broadcast::Receiver<GameState>),
    KeyReceiver(broadcast::Receiver<KeyBinding>),
    QueryCaptureHandles((Vec<String>, Option<usize>)),
    SelectCaptureHandle,
    #[cfg(debug_assertions)]
    CaptureImage,
    #[cfg(debug_assertions)]
    InferRune,
    #[cfg(debug_assertions)]
    InferMinimap,
    #[cfg(debug_assertions)]
    RecordImages,
    #[cfg(debug_assertions)]
    TestSpinRune,
}

/// Request handler of incoming requests from UI.
pub(crate) trait RequestHandler {
    fn on_rotate_actions(&mut self, halting: bool);

    fn on_create_minimap(&self, name: String) -> Option<Minimap>;

    fn on_update_minimap(&mut self, preset: Option<String>, minimap: Option<Minimap>);

    fn on_create_navigation_path(&self) -> Option<NavigationPath>;

    fn on_recapture_navigation_path(&self, path: NavigationPath) -> NavigationPath;

    fn on_update_character(&mut self, character: Option<Character>);

    fn on_redetect_minimap(&mut self);

    fn on_game_state_receiver(&self) -> broadcast::Receiver<GameState>;

    fn on_key_receiver(&self) -> broadcast::Receiver<KeyBinding>;

    fn on_query_capture_handles(&mut self) -> (Vec<String>, Option<usize>);

    fn on_select_capture_handle(&mut self, index: Option<usize>);

    #[cfg(debug_assertions)]
    fn on_capture_image(&self, is_grayscale: bool);

    #[cfg(debug_assertions)]
    fn on_infer_rune(&mut self);

    #[cfg(debug_assertions)]
    fn on_infer_minimap(&self);

    #[cfg(debug_assertions)]
    fn on_record_images(&mut self, start: bool);

    #[cfg(debug_assertions)]
    fn on_test_spin_rune(&self);
}

/// The four quads of a bound.
#[derive(Clone, Copy, Debug, Display)]
pub enum BoundQuadrant {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

/// A struct for storing game information.
#[derive(Clone, Debug)]
pub struct GameState {
    pub position: Option<(i32, i32)>,
    pub health: Option<(u32, u32)>,
    pub state: String,
    pub normal_action: Option<String>,
    pub priority_action: Option<String>,
    pub erda_shower_state: String,
    pub destinations: Vec<(i32, i32)>,
    pub operation: GameOperation,
    pub frame: Option<(Vec<u8>, usize, usize)>,
    pub platforms_bound: Option<Bound>,
    pub portals: Vec<Bound>,
    pub auto_mob_quadrant: Option<BoundQuadrant>,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum GameOperation {
    Halting,
    HaltUntil(Instant),
    Running,
    RunUntil(Instant),
}

/// Starts or stops rotating the actions.
pub async fn rotate_actions(halting: bool) {
    expect_unit_variant!(
        request(Request::RotateActions(halting)).await,
        Response::RotateActions
    )
}

/// Queries settings from the database.
pub async fn query_settings() -> Settings {
    spawn_blocking(database::query_settings).await.unwrap()
}

/// Upserts `settings` to the database.
///
/// Returns the updated [`Settings`] or original if fails.
pub async fn upsert_settings(mut settings: Settings) -> Settings {
    spawn_blocking(move || {
        let _ = database::upsert_settings(&mut settings);
        settings
    })
    .await
    .unwrap()
}

/// Queries minimaps from the database.
pub async fn query_minimaps() -> Option<Vec<Minimap>> {
    spawn_blocking(database::query_minimaps).await.unwrap().ok()
}

/// Creates a new minimap from the currently detected minimap.
///
/// This function does not insert the created minimap into the database.
pub async fn create_minimap(name: String) -> Option<Minimap> {
    expect_value_variant!(
        request(Request::CreateMinimap(name)).await,
        Response::CreateMinimap
    )
}

/// Upserts `minimap` to the database.
///
/// If `minimap` does not previously exist, a new one will be created and its `id` will
/// be updated.
///
/// Returns the updated [`Minimap`] on success.
pub async fn upsert_minimap(mut minimap: Minimap) -> Option<Minimap> {
    spawn_blocking(move || {
        database::upsert_minimap(&mut minimap)
            .is_ok()
            .then_some(minimap)
    })
    .await
    .unwrap()
}

/// Updates the current minimap used by the main game loop.
pub async fn update_minimap(preset: Option<String>, minimap: Option<Minimap>) {
    expect_unit_variant!(
        request(Request::UpdateMinimap(preset, minimap)).await,
        Response::UpdateMinimap
    )
}

/// Deletes `minimap` from the database.
///
/// Returns `true` if the minimap was deleted.
pub async fn delete_minimap(minimap: Minimap) -> bool {
    spawn_blocking(move || database::delete_minimap(&minimap).is_ok())
        .await
        .unwrap()
}

/// Queries navigation paths from the database.
pub async fn query_navigation_paths() -> Option<Vec<NavigationPath>> {
    spawn_blocking(database::query_navigation_paths)
        .await
        .unwrap()
        .ok()
}

/// Creates a navigation path from currently detected minimap.
///
/// This function does not insert the created path into the database.
pub async fn create_navigation_path() -> Option<NavigationPath> {
    expect_value_variant!(
        request(Request::CreateNavigationPath).await,
        Response::CreateNavigationPath
    )
}

/// Upserts `path` to the database.
///
/// Returns the updated [`NavigationPath`] on success.
pub async fn upsert_navigation_path(mut path: NavigationPath) -> Option<NavigationPath> {
    spawn_blocking(move || {
        database::upsert_navigation_path(&mut path)
            .is_ok()
            .then_some(path)
    })
    .await
    .unwrap()
}

/// Recaptures snapshots for the provided `path`.
///
/// Snapshots include name and minimap will be recaptured and re-assigned to the given `path` if
/// the minimap is currently detected.
///
/// Returns the updated [`NavigationPath`] or original if minimap is currently not detectable.
pub async fn recapture_navigation_path(path: NavigationPath) -> NavigationPath {
    expect_value_variant!(
        request(Request::RecaptureNavigationPath(path)).await,
        Response::RecaptureNavigationPath
    )
}

/// Deletes `path` from the database.
///
/// Returns `true` if `path` was deleted.
pub async fn delete_navigation_path(path: NavigationPath) -> bool {
    spawn_blocking(move || database::delete_navigation_path(&path).is_ok())
        .await
        .unwrap()
}

/// Queries characters from the database.
pub async fn query_characters() -> Option<Vec<Character>> {
    spawn_blocking(database::query_characters)
        .await
        .unwrap()
        .ok()
}

/// Upserts `character` to the database.
///
/// If `character` does not previously exist, a new one will be created and its `id` will
/// be updated.
///
/// Returns the updated [`Character`] on success.
pub async fn upsert_character(mut character: Character) -> Option<Character> {
    spawn_blocking(move || {
        database::upsert_character(&mut character)
            .is_ok()
            .then_some(character)
    })
    .await
    .unwrap()
}

/// Updates the current character used by the main game loop.
pub async fn update_character(character: Option<Character>) {
    expect_unit_variant!(
        request(Request::UpdateCharacter(character)).await,
        Response::UpdateCharacter
    )
}

/// Deletes `character` from the database.
///
/// Returns `true` if the `character` was deleted.
pub async fn delete_character(character: Character) -> bool {
    spawn_blocking(move || database::delete_character(&character).is_ok())
        .await
        .unwrap()
}

pub async fn redetect_minimap() {
    expect_unit_variant!(
        request(Request::RedetectMinimap).await,
        Response::RedetectMinimap
    )
}

pub async fn game_state_receiver() -> broadcast::Receiver<GameState> {
    expect_value_variant!(
        request(Request::GameStateReceiver).await,
        Response::GameStateReceiver
    )
}

pub async fn key_receiver() -> broadcast::Receiver<KeyBinding> {
    expect_value_variant!(request(Request::KeyReceiver).await, Response::KeyReceiver)
}

pub async fn query_capture_handles() -> (Vec<String>, Option<usize>) {
    expect_value_variant!(
        request(Request::QueryCaptureHandles).await,
        Response::QueryCaptureHandles
    )
}

pub async fn select_capture_handle(index: Option<usize>) {
    expect_unit_variant!(
        request(Request::SelectCaptureHandle(index)).await,
        Response::SelectCaptureHandle
    )
}

#[cfg(debug_assertions)]
pub async fn capture_image(is_grayscale: bool) {
    expect_unit_variant!(
        request(Request::CaptureImage(is_grayscale)).await,
        Response::CaptureImage
    )
}

#[cfg(debug_assertions)]
pub async fn infer_rune() {
    expect_unit_variant!(request(Request::InferRune).await, Response::InferRune)
}

#[cfg(debug_assertions)]
pub async fn infer_minimap() {
    expect_unit_variant!(request(Request::InferMinimap).await, Response::InferMinimap)
}

#[cfg(debug_assertions)]
pub async fn record_images(start: bool) {
    expect_unit_variant!(
        request(Request::RecordImages(start)).await,
        Response::RecordImages
    )
}

#[cfg(debug_assertions)]
pub async fn test_spin_rune() {
    expect_unit_variant!(request(Request::TestSpinRune).await, Response::TestSpinRune)
}

pub(crate) fn poll_request(handler: &mut dyn RequestHandler) {
    if let Ok((request, sender)) = LazyLock::force(&REQUESTS).1.lock().unwrap().try_recv() {
        let result = match request {
            Request::RotateActions(halting) => {
                handler.on_rotate_actions(halting);
                Response::RotateActions
            }
            Request::CreateMinimap(name) => {
                Response::CreateMinimap(handler.on_create_minimap(name))
            }
            Request::UpdateMinimap(preset, minimap) => {
                handler.on_update_minimap(preset, minimap);
                Response::UpdateMinimap
            }
            Request::CreateNavigationPath => {
                Response::CreateNavigationPath(handler.on_create_navigation_path())
            }
            Request::RecaptureNavigationPath(path) => {
                Response::RecaptureNavigationPath(handler.on_recapture_navigation_path(path))
            }
            Request::UpdateCharacter(character) => {
                handler.on_update_character(character);
                Response::UpdateCharacter
            }
            Request::RedetectMinimap => {
                handler.on_redetect_minimap();
                Response::RedetectMinimap
            }
            Request::GameStateReceiver => {
                Response::GameStateReceiver(handler.on_game_state_receiver())
            }
            Request::KeyReceiver => Response::KeyReceiver(handler.on_key_receiver()),
            Request::QueryCaptureHandles => {
                Response::QueryCaptureHandles(handler.on_query_capture_handles())
            }
            Request::SelectCaptureHandle(index) => {
                handler.on_select_capture_handle(index);
                Response::SelectCaptureHandle
            }
            #[cfg(debug_assertions)]
            Request::CaptureImage(is_grayscale) => {
                handler.on_capture_image(is_grayscale);
                Response::CaptureImage
            }
            #[cfg(debug_assertions)]
            Request::InferRune => {
                handler.on_infer_rune();
                Response::InferRune
            }
            #[cfg(debug_assertions)]
            Request::InferMinimap => {
                handler.on_infer_minimap();
                Response::InferMinimap
            }
            #[cfg(debug_assertions)]
            Request::RecordImages(start) => {
                handler.on_record_images(start);
                Response::RecordImages
            }
            #[cfg(debug_assertions)]
            Request::TestSpinRune => {
                handler.on_test_spin_rune();
                Response::TestSpinRune
            }
        };
        let _ = sender.send(result);
    }
}

async fn request(request: Request) -> Response {
    let (tx, rx) = oneshot::channel();
    LazyLock::force(&REQUESTS)
        .0
        .send((request, tx))
        .await
        .unwrap();
    rx.await.unwrap()
}
