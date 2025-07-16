use std::collections::{HashMap, HashSet};

use backend::{
    NavigationPath, NavigationPoint, NavigationTransition, create_navigation_path,
    delete_navigation_path, query_navigation_paths, recapture_navigation_path,
    upsert_navigation_path,
};
use dioxus::prelude::*;
use futures_util::StreamExt;

use crate::{
    AppState,
    button::{Button, ButtonKind},
    icons::{DetailsIcon, XIcon},
    select::Select,
};

#[derive(Debug, Clone, PartialEq)]
enum NavigationPopup {
    Snapshots(NavigationPath),
    Point(NavigationPath, usize),
}

#[derive(Debug)]
enum NavigationUpdate {
    Update(NavigationPath),
    Create,
    Delete(NavigationPath),
    Recapture(NavigationPath),
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
pub fn PopupSnapshots(
    name_base64: String,
    minimap_base64: String,
    on_recapture: EventHandler,
    on_close: EventHandler,
) -> Element {
    rsx! {
        div { class: "px-16 py-20 w-full h-full absolute inset-0 z-1 bg-gray-950/80 flex",
            div { class: "bg-gray-900 w-full max-w-108 h-full min-h-70 max-h-80 px-2 m-auto",
                Section {
                    name: "Path snapshots",
                    class: "relative h-full !pr-0 !pb-10",
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
                    div { class: "flex w-full gap-3 absolute bottom-0 py-2 bg-gray-900",
                        Button {
                            class: "flex-grow border border-gray-600",
                            text: "Re-capture",
                            kind: ButtonKind::Secondary,
                            on_click: move |_| {
                                on_recapture(());
                            },
                        }
                        Button {
                            class: "flex-grow border border-gray-600",
                            text: "Close",
                            kind: ButtonKind::Secondary,
                            on_click: move |_| {
                                on_close(());
                            },
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn PopupPoint(
    x: i32,
    y: i32,
    on_save: EventHandler<(i32, i32)>,
    on_close: EventHandler,
) -> Element {
    rsx! {
        div { class: "px-16 py-20 w-full h-full absolute inset-0 z-1 bg-gray-950/80 flex",
            div { class: "bg-gray-900 w-full max-w-108 h-full min-h-70 max-h-80 px-2 m-auto",
                Section {
                    name: "Path snapshots",
                    class: "relative h-full !pr-0 !pb-10",
                }
            }
        }
    }
}

#[component]
fn SectionPaths(popup: Signal<Option<NavigationPopup>>) -> Element {
    let position = use_context::<AppState>().position;
    let mut paths = use_resource(async || query_navigation_paths().await.unwrap_or_default());
    let paths_view = use_memo(move || paths().unwrap_or_default());
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

        for path in root_paths {
            visiting.push(path);

            let mut path_flattened = Vec::new();
            while let Some(path) = visiting.pop() {
                if !visited.insert(path.id) {
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

        let root_paths_flattened_id = root_paths_flattened
            .iter()
            .flat_map(|paths| paths.iter().filter_map(|path| path.id))
            .collect::<HashSet<_>>();
        let circular_paths = paths
            .iter()
            .filter(|path| {
                path.id
                    .is_some_and(|id| !root_paths_flattened_id.contains(&id))
            })
            .collect::<Vec<_>>();

        root_paths_flattened.push(circular_paths);
        root_paths_flattened
            .into_iter()
            .map(|paths| paths.into_iter().cloned().collect())
            .collect::<Vec<Vec<_>>>()
    });

    let coroutine = use_coroutine(
        move |mut rx: UnboundedReceiver<NavigationUpdate>| async move {
            while let Some(message) = rx.next().await {
                match message {
                    NavigationUpdate::Update(path) => {
                        let _ = upsert_navigation_path(path).await;
                        paths.restart();
                    }
                    NavigationUpdate::Create => {
                        let Some(path) = create_navigation_path().await else {
                            continue;
                        };
                        let _ = upsert_navigation_path(path).await;
                        paths.restart();
                    }
                    NavigationUpdate::Delete(path) => {
                        delete_navigation_path(path).await;
                        paths.restart();
                    }
                    NavigationUpdate::Recapture(path) => {
                        let new_path = recapture_navigation_path(path).await;
                        let new_path = upsert_navigation_path(new_path).await;

                        if let Some(NavigationPopup::Snapshots(path)) = popup()
                            && path.id == new_path.id
                        {
                            popup.set(Some(NavigationPopup::Snapshots(new_path)));
                        }
                        paths.restart();
                    }
                }
            }
        },
    );
    let on_add_point = use_callback::<NavigationPath, _>(move |mut path| {
        path.points.push(NavigationPoint {
            next_path_id: None,
            x: position.peek().0,
            y: position.peek().1,
            transition: NavigationTransition::Portal,
        });
        coroutine.send(NavigationUpdate::Update(path));
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
        Section { name: "Paths",
            div { class: "flex flex-col gap-2",
                for (index , paths) in root_paths_view().into_iter().enumerate() {
                    for path in paths {
                        NavigationPathItem {
                            path,
                            paths_view,
                            on_add_point: move |path| {
                                on_add_point(path);
                            },
                            on_delete_point: move |args| {
                                on_delete_point(args);
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
                        on_close: move |_| {
                            popup.set(None);
                        },
                        on_recapture: move |_| {
                            coroutine.send(NavigationUpdate::Recapture(path.clone()));
                        },
                    }
                },
                NavigationPopup::Point(path, index) => rsx! {
                    if let Some(point) = path.points.get(index) {
                        PopupPoint {
                            x: point.x,
                            y: point.y,
                            on_save: move |(x, y)| {
                                let mut path = path.clone();
                                if let Some(point) = path.points.get_mut(index) {
                                    point.x = x;
                                    point.y = y;
                                    coroutine.send(NavigationUpdate::Update(path));
                                }
                                popup.set(None);
                            },
                            on_close: move |_| {
                                popup.set(None);
                            },
                        }
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
    on_add_point: EventHandler<NavigationPath>,
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
    let path_ids = use_memo(move || {
        paths_view()
            .into_iter()
            .filter_map(|path| path.id.map(|id| format!("Path {id}")))
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
                    div { class: "grid grid-cols-[32px_auto] gap-x-2 group/info",
                        div { class: "h-full border-l-2 border-gray-600" }
                        p { class: "label h-full flex items-center justify-centers group-hover/info:border-b group-hover/info:border-gray-600",
                            {format!("X / {}, Y / {} using {}", point.x, point.y, point.transition)}
                        }
                    }

                    div { class: "grid grid-cols-2 gap-x-2",
                        Select::<String> {
                            div_class: "!gap-0",
                            options: [vec!["None".to_string()], path_ids()].concat(),
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
