use std::{
    fs::File,
    io::BufReader,
    ops::Deref,
    time::{Duration, Instant},
};

use backend::{
    Action, ActionKey, ActionMove, DatabaseEvent, GameOperation, Minimap as MinimapData, Position,
    RotationMode, create_minimap, database_event_receiver, delete_minimap, game_state_receiver,
    query_minimaps, redetect_minimap, rotate_actions, update_minimap, upsert_minimap,
};
use dioxus::{document::EvalError, prelude::*};
use futures_util::StreamExt;
use rand::distr::{Alphanumeric, SampleString};
use serde::Serialize;
use tokio::{sync::broadcast::error::RecvError, time::sleep};

use crate::{
    AppState,
    button::{Button, ButtonKind},
    select::TextSelect,
};

const BACKGROUND: Asset = asset!(
    "assets/background.png",
    ImageAssetOptions::new().with_webp()
);

const MINIMAP_JS: &str = r#"
    const canvas = document.getElementById("canvas-minimap");
    const canvasCtx = canvas.getContext("2d");

    while (true) {
        const [buffer, width, height, destinations, bound, quadrant, portals] = await dioxus.recv();
        const data = new ImageData(new Uint8ClampedArray(buffer), width, height);
        const bitmap = await createImageBitmap(data);

        canvasCtx.fillStyle = "rgb(128, 255, 204)";
        canvasCtx.strokeStyle = "rgb(128, 255, 204)";
        canvasCtx.drawImage(bitmap, 0, 0, width, height, 0, 0, canvas.width, canvas.height);

        const destinationSize = 4;
        const destinationSizeHalf = destinationSize / 2;
        let prevX = 0;
        let prevY = 0;
        for (let i = 0; i < destinations.length; i++) {
            let [x, y] = destinations[i];
            x = (x / width) * canvas.width;
            y = ((height - y) / height) * canvas.height;

            canvasCtx.fillRect(x, y, destinationSize, destinationSize);

            if (i > 0) {
                canvasCtx.beginPath();
                canvasCtx.setLineDash([8]);
                canvasCtx.moveTo(prevX + destinationSizeHalf, prevY + destinationSizeHalf);
                canvasCtx.lineTo(x + destinationSizeHalf, y + destinationSizeHalf);
                canvasCtx.stroke();
            }

            prevX = x;
            prevY = y;
        }

        canvasCtx.setLineDash([8]);
        canvasCtx.strokeStyle = "rgb(160, 155, 255)";
        for (let i = 0; i < portals.length; i++) {
            const portal = portals[i];
            const x = (portal.x / width) * canvas.width;
            const y = ((height - portal.y - portal.height) / height) * canvas.height;
            const w = (portal.width / width) * canvas.width;
            const h = (portal.height / height) * canvas.height;

            canvasCtx.strokeRect(x, y, w, h);
        }

        if (quadrant !== null && bound !== null) {
            canvasCtx.strokeStyle = "rgb(254, 71, 57)";

            const x = (bound.x / width) * canvas.width;
            const y = (bound.y / height) * canvas.height;
            const w = (bound.width / width) * canvas.width;
            const h = (bound.height / height) * canvas.height;

            const widthHalf = w / 2;
            const heightHalf = h / 2;
            const widthQuarter = widthHalf / 2;
            const heightQuarter = heightHalf / 2;

            switch (quadrant) {
                case "TopLeft": {
                    const fromX = x + widthQuarter;
                    const fromY = y + heightQuarter;
                    const toX = x + widthHalf + widthQuarter;
                    drawArrow(canvasCtx, fromX, fromY, toX, fromY);
                    break;
                }
                case "TopRight": {
                    const fromX = x + widthHalf + widthQuarter;
                    const fromY = y + heightQuarter;
                    const toY = y + heightHalf + heightQuarter;
                    drawArrow(canvasCtx, fromX, fromY, fromX, toY);
                    break;
                }
                case "BottomRight": {
                    const fromX = x + widthHalf + widthQuarter;
                    const fromY = y + heightHalf + heightQuarter;
                    const toX = x + widthQuarter;
                    drawArrow(canvasCtx, fromX, fromY, toX, fromY);
                    break;
                }
                case "BottomLeft": {
                    const fromX = x + widthQuarter;
                    const fromY = y + heightHalf + heightQuarter;
                    const toY = y + heightQuarter;
                    drawArrow(canvasCtx, fromX, fromY, fromX, toY);
                    break;
                }
                default:
                    break;
            }
        }
    }

    function drawArrow(canvasCtx, fromX, fromY, toX, toY) {
        const headSize = 10; // Length of head in pixels
        const dx = toX - fromX;
        const dy = toY - fromY;
        const angle = Math.atan2(dy, dx);

        canvasCtx.beginPath();
        canvasCtx.setLineDash([8]);
        canvasCtx.moveTo(fromX, fromY);
        canvasCtx.lineTo(toX, toY);
        canvasCtx.stroke();

        canvasCtx.beginPath();
        canvasCtx.setLineDash([]);
        canvasCtx.moveTo(toX, toY);
        canvasCtx.lineTo(toX - headSize * Math.cos(angle - Math.PI / 6), toY - headSize * Math.sin(angle - Math.PI / 6));
        canvasCtx.moveTo(toX, toY);
        canvasCtx.lineTo(toX - headSize * Math.cos(angle + Math.PI / 6), toY - headSize * Math.sin(angle + Math.PI / 6));
        canvasCtx.stroke();
    }
"#;
const MINIMAP_ACTIONS_JS: &str = r#"
    const canvas = document.getElementById("canvas-minimap-actions");
    const canvasCtx = canvas.getContext("2d");
    const [width, height, actions, boundAndType, platforms] = await dioxus.recv();
    canvasCtx.clearRect(0, 0, canvas.width, canvas.height);
    const anyActions = actions.filter((action) => action.condition === "Any");
    const erdaActions = actions.filter((action) => action.condition === "ErdaShowerOffCooldown");
    const millisActions = actions.filter((action) => action.condition === "EveryMillis");

    drawBound(canvasCtx, boundAndType);

    canvasCtx.setLineDash([]);
    canvasCtx.strokeStyle = "rgb(255, 160, 37)";
    for (const platform of platforms) {
        const xStart = (platform.x_start / width) * canvas.width;
        const xEnd = (platform.x_end / width) * canvas.width;
        const y = ((height - platform.y) / height) * canvas.height;
        canvasCtx.beginPath();
        canvasCtx.moveTo(xStart, y);
        canvasCtx.lineTo(xEnd, y);
        canvasCtx.stroke();
    }

    canvasCtx.setLineDash([8]);
    canvasCtx.fillStyle = "rgb(255, 153, 128)";
    canvasCtx.strokeStyle = "rgb(255, 153, 128)";
    drawActions(canvas, canvasCtx, anyActions, true);

    canvasCtx.fillStyle = "rgb(179, 198, 255)";
    canvasCtx.strokeStyle = "rgb(179, 198, 255)";
    drawActions(canvas, canvasCtx, erdaActions, true);

    canvasCtx.fillStyle = "rgb(128, 255, 204)";
    canvasCtx.strokeStyle = "rgb(128, 255, 204)";
    drawActions(canvas, canvasCtx, millisActions, false);

    function drawBound(canvasCtx, boundAndType) {
        if (boundAndType === null) {
            return;
        }
        const [bound, boundType] = boundAndType;
        if (bound.width === 0 || bound.height === 0) {
            return;
        }
        const x = (bound.x / width) * canvas.width;
        const y = (bound.y / height) * canvas.height;
        const w = (bound.width / width) * canvas.width;
        const h = (bound.height / height) * canvas.height;

        canvasCtx.strokeStyle = "rgb(152, 233, 32)";
        canvasCtx.beginPath();
        canvasCtx.setLineDash([8]);
        canvasCtx.strokeRect(x, y, w, h);

        if (boundType === "PingPong") {
            canvasCtx.strokeStyle = "rgb(254, 71, 57)";

            canvasCtx.moveTo(0, y);
            canvasCtx.lineTo(x - 5, y);

            canvasCtx.moveTo(0, y + h);
            canvasCtx.lineTo(x - 5, y + h);

            canvasCtx.moveTo(x + w + 5, y);
            canvasCtx.lineTo(canvas.width, y);

            canvasCtx.moveTo(x + w + 5, y + h);
            canvasCtx.lineTo(canvas.width, y + h);

            canvasCtx.moveTo(x, 0);
            canvasCtx.lineTo(x, y);

            canvasCtx.moveTo(x + w, 0);
            canvasCtx.lineTo(x + w, y);

            canvasCtx.moveTo(x, y + h);
            canvasCtx.lineTo(x, canvas.height);

            canvasCtx.moveTo(x + w, y + h);
            canvasCtx.lineTo(x + w, canvas.height);
        }
        if (boundType === "AutoMobbing") {
            canvasCtx.moveTo(x + w / 2, y + 2);
            canvasCtx.lineTo(x + w / 2, y + h - 2);
            
            canvasCtx.moveTo(x + 2, y + h / 2);
            canvasCtx.lineTo(x + w - 2, y + h / 2);
        }
        canvasCtx.stroke();
    }

    function drawActions(canvas, ctx, actions, hasArc) {
        const rectSize = 4;
        const rectHalf = rectSize / 2;
        let lastAction = null;
        let i = 1;

        ctx.font = '12px sans-serif';

        for (const action of actions) {
            const x = (action.x / width) * canvas.width;
            const y = ((height - action.y) / height) * canvas.height;

            ctx.fillRect(x, y, rectSize, rectSize);

            let labelX = x + rectSize / 2;
            let labelY = y + rectSize - 7;
            ctx.fillText(i, labelX, labelY);

            if (hasArc && lastAction !== null) {
                let [fromX, fromY] = lastAction;
                drawArc(ctx, fromX + rectHalf, fromY + rectHalf, x + rectHalf, y + rectHalf);
            }

            lastAction = [x, y];
            i++;
        }
    }
    function drawArc(ctx, fromX, fromY, toX, toY) {
        const cx = (fromX + toX) / 2;
        const cy = (fromY + toY) / 2;
        const dx = cx - fromX;
        const dy = cy - fromY;
        const radius = Math.sqrt(dx * dx + dy * dy);
        const startAngle = Math.atan2(fromY - cy, fromX - cx);
        const endAngle = Math.atan2(toY - cy, toX - cx);
        ctx.beginPath();
        ctx.arc(cx, cy, radius, startAngle, endAngle, false);
        ctx.stroke();
    }
"#;

#[derive(Clone, PartialEq, Serialize)]
struct ActionView {
    x: i32,
    y: i32,
    condition: String,
}

#[derive(PartialEq, Clone, Debug)]
struct MinimapState {
    position: Option<(i32, i32)>,
    health: Option<(u32, u32)>,
    state: String,
    normal_action: Option<String>,
    priority_action: Option<String>,
    erda_shower_state: String,
    operation: GameOperation,
    detected_size: Option<(usize, usize)>,
}

#[derive(Debug)]
enum MinimapUpdate {
    Set,
    Create(String),
    Import(MinimapData),
    Delete,
}

#[component]
pub fn Minimap() -> Element {
    let mut minimap = use_context::<AppState>().minimap;
    let mut minimap_preset = use_context::<AppState>().minimap_preset;
    let mut minimaps = use_resource(async || query_minimaps().await.unwrap_or_default());
    let position = use_context::<AppState>().position;
    // Maps queried `minimaps` to names
    let minimap_names = use_memo(move || {
        minimaps()
            .unwrap_or_default()
            .into_iter()
            .map(|minimap| minimap.name)
            .collect()
    });
    // Maps currently selected `minimap` to the index in `minimaps`
    let minimap_index = use_memo(move || {
        minimaps().zip(minimap()).and_then(|(minimaps, minimap)| {
            minimaps
                .into_iter()
                .enumerate()
                .find(|(_, data)| minimap.id == data.id)
                .map(|(i, _)| i)
        })
    });

    // Game state for displaying info
    let state = use_signal::<Option<MinimapState>>(|| None);
    // Handles async operations for minimap-related
    let coroutine = use_coroutine(move |mut rx: UnboundedReceiver<MinimapUpdate>| async move {
        while let Some(message) = rx.next().await {
            match message {
                MinimapUpdate::Set => {
                    update_minimap(minimap_preset(), minimap()).await;
                }
                MinimapUpdate::Create(name) => {
                    let Some(new_minimap) = create_minimap(name).await else {
                        continue;
                    };
                    let Some(new_minimap) = upsert_minimap(new_minimap).await else {
                        continue;
                    };

                    minimap.set(Some(new_minimap));
                    minimap_preset.set(None);
                    update_minimap(None, minimap()).await;
                }
                MinimapUpdate::Import(minimap) => {
                    upsert_minimap(minimap).await;
                }
                MinimapUpdate::Delete => {
                    if let Some(current_minimap) = minimap()
                        && delete_minimap(current_minimap).await
                    {
                        minimap.set(None);
                        minimap_preset.set(None);
                    }
                }
            }
        }
    });

    // Sets a minimap and preset if there is not one
    use_effect(move || {
        if let Some(minimaps) = minimaps()
            && !minimaps.is_empty()
            && minimap.peek().is_none()
        {
            minimap.set(minimaps.into_iter().next());
            minimap_preset.set(
                minimap
                    .peek()
                    .as_ref()
                    .expect("has value")
                    .actions
                    .keys()
                    .next()
                    .cloned(),
            );
            coroutine.send(MinimapUpdate::Set);
        }
    });
    // External modification checking
    use_future(move || async move {
        let mut rx = database_event_receiver();
        loop {
            let event = match rx.recv().await {
                Ok(value) => value,
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            };
            if matches!(
                event,
                DatabaseEvent::MinimapUpdated(_) | DatabaseEvent::MinimapDeleted(_)
            ) {
                minimaps.restart();
            }
        }
    });

    rsx! {
        div { class: "relative flex flex-col flex-none w-xs xl:w-md z-0",
            div {
                class: "absolute inset-0 bg-no-repeat bg-center w-[130%] -z-1",
                style: "background-image: url({BACKGROUND}); background-size: 150%; background-position: 85% 70px;",
            }
            Canvas {
                state,
                minimap,
                minimap_preset,
                position,
            }
            Buttons { state, minimap }
            Info { state, minimap }
            div { class: "flex-grow flex items-end px-2",
                div { class: "flex flex-col items-end w-full",
                    ImportExport { minimap }
                    div { class: "h-10 w-full flex items-center",
                        TextSelect {
                            class: "w-full",
                            options: minimap_names(),
                            disabled: false,
                            placeholder: "Create a map...",
                            on_create: move |name| {
                                coroutine.send(MinimapUpdate::Create(name));
                            },
                            on_delete: move |_| {
                                coroutine.send(MinimapUpdate::Delete);
                            },
                            on_select: move |(index, _)| {
                                let selected: MinimapData = minimaps
                                    .peek()
                                    .as_ref()
                                    .expect("should already loaded")
                                    .get(index)
                                    .cloned()
                                    .unwrap();
                                minimap_preset.set(selected.actions.keys().next().cloned());
                                minimap.set(Some(selected));
                                coroutine.send(MinimapUpdate::Set);
                            },
                            selected: minimap_index(),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Canvas(
    state: Signal<Option<MinimapState>>,
    minimap: ReadOnlySignal<Option<MinimapData>>,
    minimap_preset: ReadOnlySignal<Option<String>>,
    position: Signal<(i32, i32)>,
) -> Element {
    let mut platforms_bound = use_signal(|| None);
    let rotation_bound_and_type = use_memo(move || {
        let platforms_bound = platforms_bound();
        let minimap = minimap()?;

        match minimap.rotation_mode {
            RotationMode::StartToEnd | RotationMode::StartToEndThenReverse => None,
            RotationMode::AutoMobbing => Some((
                platforms_bound.unwrap_or(minimap.rotation_auto_mob_bound),
                "AutoMobbing",
            )),
            RotationMode::PingPong => Some((minimap.rotation_ping_pong_bound, "PingPong")),
        }
    });

    use_effect(move || {
        let bound_and_type = rotation_bound_and_type();
        let preset = minimap_preset();
        let Some(minimap) = minimap() else {
            return;
        };
        let actions = preset
            .and_then(|preset| minimap.actions.get(&preset).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|action| match action {
                Action::Move(ActionMove {
                    position: Position { x, y, .. },
                    condition,
                    ..
                })
                | Action::Key(ActionKey {
                    position: Some(Position { x, y, .. }),
                    condition,
                    ..
                }) => Some(ActionView {
                    x,
                    y,
                    condition: condition.to_string(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();

        spawn(async move {
            let canvas = document::eval(MINIMAP_ACTIONS_JS);
            let _ = canvas.send((
                minimap.width,
                minimap.height,
                actions,
                bound_and_type,
                minimap.platforms,
            ));
        });
    });
    // Draw minimap and update game state
    use_future(move || async move {
        let mut canvas = document::eval(MINIMAP_JS);
        let mut receiver = game_state_receiver().await;
        loop {
            let Ok(current_state) = receiver.recv().await else {
                continue;
            };
            let destinations = current_state.destinations;
            let bound = current_state.platforms_bound;
            let quadrant = current_state
                .auto_mob_quadrant
                .map(|quadrant| quadrant.to_string());
            let frame = current_state.frame;
            let portals = current_state.portals;
            let current_state = MinimapState {
                position: current_state.position,
                health: current_state.health,
                state: current_state.state,
                normal_action: current_state.normal_action,
                priority_action: current_state.priority_action,
                erda_shower_state: current_state.erda_shower_state,
                operation: current_state.operation,
                detected_size: frame.as_ref().map(|(_, width, height)| (*width, *height)),
            };

            if *platforms_bound.peek() != bound {
                platforms_bound.set(bound);
            }
            if *position.peek() != current_state.position.unwrap_or_default() {
                position.set(current_state.position.unwrap_or_default());
            }
            state.set(Some(current_state));
            sleep(Duration::from_millis(50)).await;

            let bound = rotation_bound_and_type
                .peek()
                .deref()
                .map(|(bound, _)| bound);
            let Some((frame, width, height)) = frame else {
                continue;
            };
            let Err(error) =
                canvas.send((frame, width, height, destinations, bound, quadrant, portals))
            else {
                continue;
            };
            if matches!(error, EvalError::Finished) {
                // probably: https://github.com/DioxusLabs/dioxus/issues/2979
                canvas = document::eval(MINIMAP_JS);
            }
        }
    });

    rsx! {
        div { class: "relative h-31 xl:h-38 rounded-2xl bg-gray-900",
            canvas {
                class: "absolute inset-0 rounded-2xl w-full h-full",
                id: "canvas-minimap",
            }
            canvas {
                class: "absolute inset-0 rounded-2xl w-full h-full",
                id: "canvas-minimap-actions",
            }
        }
    }
}

#[component]
fn Info(
    state: ReadOnlySignal<Option<MinimapState>>,
    minimap: ReadOnlySignal<Option<MinimapData>>,
) -> Element {
    #[derive(Debug, PartialEq, Clone)]
    struct GameStateInfo {
        position: String,
        health: String,
        state: String,
        normal_action: String,
        priority_action: String,
        erda_shower_state: String,
        detected_minimap_size: String,
        selected_minimap_size: String,
        cycle_duration: String,
    }

    let info = use_memo(move || {
        let mut info = GameStateInfo {
            position: "Unknown".to_string(),
            health: "Unknown".to_string(),
            state: "Unknown".to_string(),
            normal_action: "None".to_string(),
            priority_action: "None".to_string(),
            erda_shower_state: "Unknown".to_string(),
            detected_minimap_size: "Unknown".to_string(),
            selected_minimap_size: "Unknown".to_string(),
            cycle_duration: "None".to_string(),
        };

        if let Some(minimap) = minimap() {
            info.selected_minimap_size = format!("{}px x {}px", minimap.width, minimap.height);
        }

        if let Some(state) = state() {
            info.state = state.state;
            info.erda_shower_state = state.erda_shower_state;
            info.cycle_duration = match state.operation {
                GameOperation::Halting | GameOperation::Running => "None".to_string(),
                GameOperation::HaltUntil(instant) | GameOperation::RunUntil(instant) => {
                    let duration = instant.saturating_duration_since(Instant::now());
                    let seconds = duration.as_secs() % 60;
                    let minutes = (duration.as_secs() / 60) % 60;
                    let hours = (duration.as_secs() / 60) / 60;
                    format!("{hours:0>2}:{minutes:0>2}:{seconds:0>2}")
                }
            };
            if let Some((x, y)) = state.position {
                info.position = format!("{x}, {y}");
            }
            if let Some((current, max)) = state.health {
                info.health = format!("{current} / {max}");
            }
            if let Some(action) = state.normal_action {
                info.normal_action = action;
            }
            if let Some(action) = state.priority_action {
                info.priority_action = action;
            }
            if let Some((width, height)) = state.detected_size {
                info.detected_minimap_size = format!("{width}px x {height}px")
            }
        }

        info
    });

    rsx! {
        div { class: "grid grid-cols-2 items-center justify-center px-4 py-3 gap-1",
            InfoItem { name: "State", value: info().state }
            InfoItem { name: "Position", value: info().position }
            InfoItem { name: "Health", value: info().health }
            InfoItem { name: "Priority action", value: info().priority_action }
            InfoItem { name: "Normal action", value: info().normal_action }
            InfoItem { name: "Erda Shower", value: info().erda_shower_state }
            InfoItem { name: "Detected size", value: info().detected_minimap_size }
            InfoItem { name: "Selected size", value: info().selected_minimap_size }
            InfoItem { name: "Run/stop cycle", value: info().cycle_duration }
        }
    }
}

#[component]
fn InfoItem(name: String, value: String) -> Element {
    rsx! {
        p { class: "paragraph font-mono", "{name}" }
        p { class: "paragraph text-right font-mono", "{value}" }
    }
}

#[component]
fn Buttons(
    state: ReadOnlySignal<Option<MinimapState>>,
    minimap: ReadOnlySignal<Option<MinimapData>>,
) -> Element {
    let halting = use_memo(move || {
        state()
            .map(|state| matches!(state.operation, GameOperation::Halting))
            .unwrap_or_default()
    });
    let character = use_context::<AppState>().character;

    rsx! {
        div { class: "flex h-10 justify-center items-center gap-4",
            Button {
                class: "w-20",
                text: if halting() { "Start" } else { "Stop" },
                kind: ButtonKind::Primary,
                disabled: minimap().is_none() || character().is_none(),
                on_click: move || async move {
                    rotate_actions(!*halting.peek()).await;
                },
            }
            Button {
                class: "w-20",
                text: "Re-detect",
                kind: ButtonKind::Primary,
                on_click: move |_| async move {
                    redetect_minimap().await;
                },
            }
        }
    }
}

#[component]
fn ImportExport(minimap: ReadOnlySignal<Option<MinimapData>>) -> Element {
    let coroutine = use_coroutine_handle::<MinimapUpdate>();
    let export_element_id = use_memo(|| Alphanumeric.sample_string(&mut rand::rng(), 8));
    let export = use_callback(move |_| {
        let js = format!(
            r#"
            const element = document.getElementById("{}");
            if (element === null) {{
                return;
            }}
            const json = await dioxus.recv();

            element.setAttribute("href", "data:application/json;charset=utf-8," + encodeURIComponent(json));
            element.setAttribute("download", "character.json");
            element.click();
            "#,
            export_element_id(),
        );
        let eval = document::eval(js.as_str());
        let Some(minimap) = &*minimap.peek() else {
            return;
        };
        let Ok(json) = serde_json::to_string_pretty(&minimap) else {
            return;
        };
        let _ = eval.send(json);
    });

    let import_element_id = use_memo(|| Alphanumeric.sample_string(&mut rand::rng(), 8));
    let import = use_callback(move |_| {
        let js = format!(
            r#"
            const element = document.getElementById("{}");
            if (element === null) {{
                return;
            }}
            element.click();
            "#,
            import_element_id()
        );
        document::eval(js.as_str());
    });
    let import_minimaps = use_callback(move |files| {
        for file in files {
            let Ok(file) = File::open(file) else {
                continue;
            };
            let reader = BufReader::new(file);
            let Ok(minimap) = serde_json::from_reader::<_, MinimapData>(reader) else {
                continue;
            };
            coroutine.send(MinimapUpdate::Import(minimap));
        }
    });

    rsx! {
        div { class: "flex gap-3",
            div {
                input {
                    id: import_element_id(),
                    class: "w-0 h-0 invisible",
                    r#type: "file",
                    accept: ".json",
                    name: "Minimap JSON",
                    onchange: move |e| {
                        if let Some(files) = e.data.files().map(|engine| engine.files()) {
                            import_minimaps(files);
                        }
                    },
                }
                Button {
                    class: "w-20",
                    text: "Import",
                    kind: ButtonKind::Primary,
                    on_click: move |_| {
                        import(());
                    },
                }
            }
            div {
                a { id: export_element_id(), class: "w-0 h-0 invisible" }
                Button {
                    class: "w-20",
                    text: "Export",
                    kind: ButtonKind::Primary,
                    disabled: minimap().is_none(),
                    on_click: move |_| {
                        export(());
                    },
                }
            }
        }
    }
}
