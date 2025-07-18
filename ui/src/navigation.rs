use std::collections::{HashMap, HashSet};

use backend::{
    NavigationPath, NavigationPoint, NavigationTransition, create_navigation_path,
    delete_navigation_path, query_navigation_paths, recapture_navigation_path, update_minimap,
    update_navigation_path, upsert_minimap, upsert_navigation_path,
};
use dioxus::prelude::*;
use futures_util::StreamExt;

use crate::{
    AppState,
    button::{Button, ButtonKind},
    icons::{DetailsIcon, PositionIcon, XIcon},
    inputs::NumberInputI32,
    popup::Popup,
    select::Select,
};

#[derive(Debug, Clone, PartialEq)]
enum NavigationPopup {
    Snapshots(NavigationPath),
    Point(NavigationPath, PopupPointValue),
}

#[derive(Debug, Clone, PartialEq)]
enum PopupPointValue {
    Add(NavigationPoint),
    Edit(NavigationPoint, usize),
}

#[derive(Debug)]
enum NavigationUpdate {
    Update(NavigationPath),
    Create,
    Delete(NavigationPath),
    Recapture(NavigationPath),
    Attach(Option<i64>),
}

#[component]
pub fn Navigation() -> Element {
    let popup = use_signal(|| None);

    rsx! {
        div { class: "flex flex-col h-full overflow-y-auto scrollbar",
            SectionPaths { popup }
        }
    }
}

#[component]
fn PopupSnapshots(
    name_base64: String,
    minimap_base64: String,
    on_recapture: EventHandler,
    on_cancel: EventHandler,
) -> Element {
    rsx! {
        Popup {
            title: "Path snapshots",
            class: "max-w-108 min-h-70 max-h-80",
            confirm_button: "Re-capture",
            on_confirm: move |_| {
                on_recapture(());
            },
            cancel_button: "Cancel",
            on_cancel: move |_| {
                on_cancel(());
            },
            div { class: "flex flex-col gap-2 pr-2 overflow-y-auto scrollbar",
                p { class: "paragraph-xs", "Name" }
                img {
                    src: format!("data:image/png;base64,{}", name_base64),
                    class: "w-full h-full p-2 border border-gray-600",
                }
                p { class: "paragraph-xs", "Map" }
                img {
                    src: format!("data:image/png;base64,{}", minimap_base64),
                    class: "w-full h-full p-2 border border-gray-600",
                }
            }
        }
    }
}

#[component]
fn PopupPoint(
    value: PopupPointValue,
    on_save: EventHandler<PopupPointValue>,
    on_close: EventHandler,
) -> Element {
    const ICON_CONTAINER_CLASS: &str = "absolute invisible group-hover:visible top-5 right-1 w-4 h-6 flex justify-center items-center";
    const ICON_CLASS: &str = "w-3 h-3 text-gray-50 fill-current";

    let position = use_context::<AppState>().position;
    let value = use_memo(use_reactive!(|value| value));
    let mut xy = use_signal(|| match value() {
        PopupPointValue::Add(point) => (point.x, point.y),
        PopupPointValue::Edit(point, _) => (point.x, point.y),
    });
    let on_save_click = use_callback(move |_| {
        let (x, y) = *xy.peek();
        let value = match value.peek().clone() {
            PopupPointValue::Add(point) => PopupPointValue::Add(NavigationPoint { x, y, ..point }),
            PopupPointValue::Edit(point, index) => {
                PopupPointValue::Edit(NavigationPoint { x, y, ..point }, index)
            }
        };
        on_save(value);
    });

    rsx! {
        Popup {
            title: "Point",
            class: "max-w-80 min-h-35 max-h-35",
            confirm_button: "Save",
            on_confirm: move |_| {
                on_save_click(());
            },
            cancel_button: "Cancel",
            on_cancel: move |_| {
                on_close(());
            },
            div { class: "grid grid-cols-2 gap-2",
                div { class: "relative group",
                    NumberInputI32 {
                        label: "X",
                        on_value: move |x| {
                            xy.write().0 = x;
                        },
                        value: xy().0,
                    }
                    div {
                        class: ICON_CONTAINER_CLASS,
                        onclick: move |_| {
                            xy.write().0 = position.peek().0;
                        },
                        PositionIcon { class: ICON_CLASS }
                    }
                }
                div { class: "relative group",
                    NumberInputI32 {
                        label: "Y",
                        on_value: move |y| {
                            xy.write().1 = y;
                        },
                        value: xy().1,
                    }
                    div {
                        class: ICON_CONTAINER_CLASS,
                        onclick: move |_| {
                            xy.write().1 = position.peek().1;
                        },
                        PositionIcon { class: ICON_CLASS }
                    }
                }
            }
        }
    }
}

#[component]
fn SectionPaths(popup: Signal<Option<NavigationPopup>>) -> Element {
    let position = use_context::<AppState>().position;
    let mut minimap = use_context::<AppState>().minimap;
    let minimap_preset = use_context::<AppState>().minimap_preset;
    let mut paths = use_resource(async || query_navigation_paths().await.unwrap_or_default());
    let paths_view = use_memo(move || paths().unwrap_or_default());
    let path_ids_view = use_memo(move || {
        paths_view()
            .into_iter()
            .filter_map(|path| path.id.map(|id| format!("Path {id}")))
            .collect::<Vec<_>>()
    });
    let minimap_attached_path_index = use_memo(move || {
        let minimap = minimap();
        let paths = paths_view();
        minimap.and_then(|minimap| minimap.path_id).and_then(|id| {
            paths.into_iter().enumerate().find_map(|(index, path)| {
                if path.id == Some(id) {
                    Some(index)
                } else {
                    None
                }
            })
        })
    });
    let mut circular_error_message = use_signal(|| None);
    // Group paths by root for better experience
    let root_paths_view = use_memo(move || {
        let paths = paths_view();
        let all_path_ids = paths
            .iter()
            .map(|path| path.id.expect("valid id"))
            .collect::<HashSet<_>>();
        let referenced_path_ids = paths
            .iter()
            .flat_map(|point| &point.points)
            .filter_map(|point| point.next_path_id)
            .collect::<HashSet<_>>();
        let root_path_ids = all_path_ids
            .difference(&referenced_path_ids)
            .copied()
            .collect::<HashSet<_>>();

        let path_by_id = paths
            .iter()
            .map(|path| (path.id.expect("valid id"), path))
            .collect::<HashMap<_, _>>();
        let root_paths = paths
            .iter()
            .filter(|path| path.id.is_some_and(|id| root_path_ids.contains(&id)))
            .collect::<Vec<_>>();

        let mut visited = HashSet::new();
        let mut visiting = Vec::new();
        let mut root_paths_flattened = Vec::new();
        let mut root_paths_circular_ids = HashSet::new();

        for path in root_paths {
            visiting.push(path);

            let mut path_flattened = Vec::new();
            while let Some(path) = visiting.pop() {
                let path_id = path.id.expect("valid id");
                if !visited.insert(path_id) {
                    root_paths_circular_ids.insert(path_id);
                    continue;
                }

                path_flattened.push(path);
                for point in path.points.iter() {
                    if let Some(path) = point.next_path_id.and_then(|id| path_by_id.get(&id)) {
                        visiting.push(path);
                    }
                }
            }

            root_paths_flattened.push(path_flattened);
        }

        let root_paths_flattened_ids = root_paths_flattened
            .iter()
            .flat_map(|paths| paths.iter().filter_map(|path| path.id))
            .collect::<HashSet<_>>();
        let circular_paths = paths
            .iter()
            .filter(|path| {
                path.id
                    .is_some_and(|id| !root_paths_flattened_ids.contains(&id))
            })
            .collect::<Vec<_>>();

        if !circular_paths.is_empty() || !root_paths_circular_ids.is_empty() {
            let path_ids = circular_paths
                .iter()
                .map(|path| path.id.expect("valid id"))
                .chain(root_paths_circular_ids)
                .map(|id| format!("Path {id}"))
                .intersperse(", ".to_string())
                .collect::<String>();
            circular_error_message.set(Some(format!("Circular paths detected in {path_ids}")));
            root_paths_flattened.push(circular_paths);
        } else {
            circular_error_message.set(None);
        }
        root_paths_flattened
            .into_iter()
            .map(|paths| paths.into_iter().cloned().collect())
            .collect::<Vec<Vec<_>>>()
    });

    let coroutine = use_coroutine(
        move |mut rx: UnboundedReceiver<NavigationUpdate>| async move {
            let update_and_restart = move || async move {
                update_navigation_path().await;
                paths.restart();
            };
            while let Some(message) = rx.next().await {
                match message {
                    NavigationUpdate::Update(path) => {
                        let _ = upsert_navigation_path(path).await;
                        update_and_restart().await;
                    }
                    NavigationUpdate::Create => {
                        let Some(path) = create_navigation_path().await else {
                            continue;
                        };
                        let _ = upsert_navigation_path(path).await;
                        update_and_restart().await;
                    }
                    NavigationUpdate::Delete(path) => {
                        delete_navigation_path(path).await;
                        update_and_restart().await;
                    }
                    NavigationUpdate::Recapture(path) => {
                        let new_path = recapture_navigation_path(path).await;
                        let new_path = upsert_navigation_path(new_path).await;

                        if let Some(NavigationPopup::Snapshots(path)) = popup()
                            && path.id == new_path.id
                        {
                            popup.set(Some(NavigationPopup::Snapshots(new_path)));
                        }
                        update_and_restart().await;
                    }
                    NavigationUpdate::Attach(path_id) => {
                        let Some(mut current_minimap) = minimap() else {
                            continue;
                        };
                        current_minimap.path_id = path_id;
                        current_minimap = upsert_minimap(current_minimap).await;

                        minimap.set(Some(current_minimap));
                        update_minimap(minimap_preset(), minimap()).await;
                    }
                }
            }
        },
    );
    let on_add_point = use_callback::<NavigationPath, _>(move |path| {
        popup.set(Some(NavigationPopup::Point(
            path,
            PopupPointValue::Add(NavigationPoint {
                next_path_id: None,
                x: position.peek().0,
                y: position.peek().1,
                transition: NavigationTransition::Portal,
            }),
        )));
    });
    let on_delete_point = use_callback::<(NavigationPath, usize), _>(move |(mut path, index)| {
        if path.points.get(index).is_some() {
            path.points.remove(index);
            coroutine.send(NavigationUpdate::Update(path));
        }
    });
    let on_select_path = use_callback::<(NavigationPath, usize, Option<i64>), _>(
        move |(mut path, point_index, next_path_id)| {
            if let Some(point) = path.points.get_mut(point_index) {
                point.next_path_id = next_path_id;
                coroutine.send(NavigationUpdate::Update(path));
            }
        },
    );

    rsx! {
        Section { name: "Selected map",
            Select {
                label: "Attached path",
                disabled: minimap().is_none(),
                options: [vec!["None".to_string()], path_ids_view()].concat(),
                on_select: move |(path_index, _)| {
                    let path_id = if path_index == 0 {
                        None
                    } else {
                        let index = path_index - 1;
                        let paths = paths_view.peek();
                        paths.get(index).and_then(|path: &NavigationPath| path.id)
                    };
                    coroutine.send(NavigationUpdate::Attach(path_id));
                },
                selected: minimap_attached_path_index().unwrap_or_default(),
            }
        }
        Section { name: "Paths",
            if let Some(message) = circular_error_message() {
                p { class: "label mb-2", "{message}" }
            }
            div { class: "flex flex-col gap-3",
                for (index , paths) in root_paths_view().into_iter().enumerate() {
                    for path in paths {
                        NavigationPathItem {
                            path,
                            paths_view,
                            path_ids_view,
                            on_add_point: move |path| {
                                on_add_point(path);
                            },
                            on_delete_point: move |args| {
                                on_delete_point(args);
                            },
                            on_edit_point: move |(path, point, index)| {
                                let edit = PopupPointValue::Edit(point, index);
                                let point = NavigationPopup::Point(path, edit);
                                popup.set(Some(point));
                            },
                            on_select_path: move |args| {
                                on_select_path(args);
                            },
                            on_delete: move |path| {
                                coroutine.send(NavigationUpdate::Delete(path));
                            },
                            on_details: move |path: NavigationPath| {
                                popup.set(Some(NavigationPopup::Snapshots(path)));
                            },
                        }
                    }
                    if index != root_paths_view.peek().len() - 1 {
                        div { class: "border-b border-dashed border-gray-600 my-2" }
                    }
                }
            }
            Button {
                text: "Add path",
                kind: ButtonKind::Secondary,
                on_click: move |_| {
                    coroutine.send(NavigationUpdate::Create);
                },
                class: "label mt-4",
            }
        }
        if let Some(kind) = popup() {
            match kind {
                NavigationPopup::Snapshots(path) => rsx! {
                    PopupSnapshots {
                        name_base64: path.name_snapshot_base64.clone(),
                        minimap_base64: path.minimap_snapshot_base64.clone(),
                        on_recapture: move |_| {
                            coroutine.send(NavigationUpdate::Recapture(path.clone()));
                        },
                        on_cancel: move |_| {
                            popup.set(None);
                        },
                    }
                },
                NavigationPopup::Point(path, value) => rsx! {
                    PopupPoint {
                        value,
                        on_save: move |value| {
                            let mut path = path.clone();
                            match value {
                                PopupPointValue::Add(point) => {
                                    path.points.push(point);
                                }
                                PopupPointValue::Edit(new_point, index) => {
                                    if let Some(point) = path.points.get_mut(index) {
                                        *point = new_point;
                                    }
                                }
                            }
                            coroutine.send(NavigationUpdate::Update(path));
                            popup.set(None);
                        },
                        on_close: move |_| {
                            popup.set(None);
                        },
                    }
                },
            }
        }
    }
}

// TODO: Whether to give a cloned path in the callbacks or let caller clone. NavigationPath
//       does not implement Copy so it is kind of inconvenient right now.
#[component]
fn NavigationPathItem(
    path: NavigationPath,
    paths_view: Memo<Vec<NavigationPath>>,
    path_ids_view: Memo<Vec<String>>,
    on_add_point: EventHandler<NavigationPath>,
    on_edit_point: EventHandler<(NavigationPath, NavigationPoint, usize)>,
    on_delete_point: EventHandler<(NavigationPath, usize)>,
    on_select_path: EventHandler<(NavigationPath, usize, Option<i64>)>,
    on_delete: EventHandler<NavigationPath>,
    on_details: EventHandler<NavigationPath>,
) -> Element {
    #[component]
    fn Icons(on_details: Option<EventHandler>, on_delete: EventHandler) -> Element {
        const ICON_CONTAINER_CLASS: &str = "w-4 h-6 flex justify-center items-center";
        const ICON_CLASS: &str = "fill-current";

        rsx! {
            div { class: "invisible group-hover:visible flex gap-1",
                div { class: "flex-grow" }
                if let Some(on_details) = on_details {
                    div {
                        class: ICON_CONTAINER_CLASS,
                        onclick: move |e| {
                            e.stop_propagation();
                            on_details(());
                        },
                        DetailsIcon { class: "{ICON_CLASS} w-[16px] h-[16px] text-gray-50" }
                    }
                }
                div {
                    class: ICON_CONTAINER_CLASS,
                    onclick: move |e| {
                        e.stop_propagation();
                        on_delete(());
                    },
                    XIcon { class: "{ICON_CLASS} w-[11px] h-[11px] text-red-500" }
                }
            }
        }
    }

    let path = use_memo(use_reactive!(|path| path));
    let paths_view = use_memo(move || {
        let path_id = path().id;
        paths_view()
            .into_iter()
            .filter(|path| path.id != path_id)
            .collect::<Vec<_>>()
    });

    rsx! {
        div {
            div { class: "grid grid-cols-2 gap-x-3 group",
                div { class: "border-b border-gray-600 p-1",
                    img {
                        width: path().name_snapshot_width,
                        height: path().name_snapshot_height,
                        src: format!("data:image/png;base64,{}", path().name_snapshot_base64),
                    }
                }
                div { class: "grid grid-cols-2 gap-x-2 group",
                    p { class: "paragraph-xs flex items-center border-b border-gray-600",
                        {format!("Path {}", path().id.unwrap_or_default())}
                    }
                    Icons {
                        on_details: move |_| {
                            on_details(path.peek().clone());
                        },
                        on_delete: move |_| {
                            on_delete(path.peek().clone());
                        },
                    }
                }
            }

            for (index , point) in path().points.into_iter().enumerate() {
                div { class: "grid grid-cols-2 gap-x-3 group mt-2",
                    div {
                        class: "grid grid-cols-[32px_auto] gap-x-2 group/info",
                        onclick: move |_| {
                            on_edit_point((path.peek().clone(), point, index));
                        },
                        div { class: "h-full border-l-2 border-gray-600" }
                        p { class: "label h-full flex items-center justify-centers group-hover/info:border-b group-hover/info:border-gray-600",
                            {format!("X / {}, Y / {} using {}", point.x, point.y, point.transition)}
                        }
                    }

                    div { class: "grid grid-cols-2 gap-x-2",
                        Select::<String> {
                            div_class: "!gap-0",
                            options: [vec!["None".to_string()], path_ids_view()].concat(),
                            on_select: move |(path_index, _)| {
                                let next_path_id = if path_index == 0 {
                                    None
                                } else {
                                    let index = path_index - 1;
                                    let paths = paths_view.peek();
                                    paths.get(index).and_then(|path: &NavigationPath| path.id)
                                };
                                on_select_path((path.peek().clone(), index, next_path_id));
                            },
                            selected: if let Some(id) = point.next_path_id { paths_view()
                                .iter()
                                .enumerate()
                                .find_map(|(index, path)| {
                                    if path.id == Some(id) { Some(index + 1) } else { None }
                                })
                                .unwrap_or_default() } else { 0 },
                        }
                        Icons {
                            on_delete: move |_| {
                                on_delete_point((path.peek().clone(), index));
                            },
                        }
                    }
                }
            }
            div { class: "grid grid-cols-2 gap-x-3 mt-2",
                Button {
                    text: "Add point",
                    kind: ButtonKind::Secondary,
                    on_click: move |_| {
                        on_add_point(path.peek().clone());
                    },
                    class: "label",
                }
                div {}
            }
        }
    }
}

#[component]
fn Section(
    name: &'static str,
    #[props(default = String::default())] class: String,
    children: Element,
) -> Element {
    rsx! {
        div { class: "flex flex-col pr-4 pb-3 {class}",
            div { class: "flex items-center title-xs h-10", {name} }
            {children}
        }
    }
}
