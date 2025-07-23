use backend::{
    DatabaseEvent, NavigationPath, NavigationPaths, NavigationPoint, NavigationTransition,
    create_navigation_path, database_event_receiver, delete_navigation_paths,
    query_navigation_paths, recapture_navigation_path, upsert_minimap, upsert_navigation_paths,
};
use dioxus::prelude::*;
use futures_util::StreamExt;
use tokio::sync::broadcast::error::RecvError;

use crate::{
    AppState,
    button::{Button, ButtonKind},
    icons::{DetailsIcon, PositionIcon, XIcon},
    inputs::NumberInputI32,
    popup::Popup,
    select::{Select, TextSelect},
};

type PathsIdIndex = Option<(i64, usize)>;

#[derive(Debug, Clone, PartialEq)]
enum NavigationPopup {
    Snapshots(NavigationPath, usize),
    Point(NavigationPath, usize, PopupPointValue),
}

#[derive(Debug, Clone, PartialEq)]
enum PopupPointValue {
    Add(NavigationPoint),
    Edit(NavigationPoint, usize),
}

#[derive(Debug)]
enum NavigationUpdate {
    Update(NavigationPaths),
    Create(String),
    Delete,
    Attach(PathsIdIndex),
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

    let mut paths = use_resource(async || query_navigation_paths().await.unwrap_or_default());
    let paths_view = use_memo(move || paths().unwrap_or_default());
    let paths_names_view = use_memo(move || {
        paths_view()
            .into_iter()
            .map(|paths| paths.name)
            .collect::<Vec<_>>()
    });

    let mut selected_paths = use_signal(move || None);
    let selected_paths_index = use_memo(move || {
        selected_paths().and_then(|paths: NavigationPaths| {
            paths_view()
                .into_iter()
                .enumerate()
                .find_map(|(index, paths_view)| (paths_view.id == paths.id).then_some(index))
        })
    });

    let minimap_paths_id_index =
        use_memo(move || minimap().and_then(|minimap| minimap.paths_id_index));
    let minimap_paths_index = use_memo(move || {
        let paths = paths_view();
        minimap_paths_id_index().and_then(|(id, _)| {
            paths.into_iter().enumerate().find_map(|(index, path)| {
                if path.id == Some(id) {
                    Some(index + 1) // + 1 for "None"
                } else {
                    None
                }
            })
        })
    });
    let minimap_paths_index_options = use_memo(move || {
        minimap_paths_index()
            .map(|index| index - 1)
            .and_then(|index| {
                paths_view
                    .peek()
                    .get(index)
                    .map(|paths| 0..paths.paths.len())
            })
            .unwrap_or_default()
            .map(|index| format!("Path {}", index + 1))
            .collect::<Vec<_>>()
    });

    let coroutine = use_coroutine(
        move |mut rx: UnboundedReceiver<NavigationUpdate>| async move {
            while let Some(message) = rx.next().await {
                match message {
                    NavigationUpdate::Update(paths) => {
                        if let Some(paths) = upsert_navigation_paths(paths).await {
                            selected_paths.set(Some(paths));
                        };
                    }
                    NavigationUpdate::Create(name) => {
                        let paths = NavigationPaths {
                            name,
                            ..NavigationPaths::default()
                        };
                        if let Some(paths) = upsert_navigation_paths(paths).await {
                            selected_paths.set(Some(paths));
                        };
                    }
                    NavigationUpdate::Delete => {
                        let Some(paths) = selected_paths() else {
                            continue;
                        };

                        if delete_navigation_paths(paths).await {
                            selected_paths.set(None);
                        }
                    }
                    NavigationUpdate::Attach(paths_id_index) => {
                        let Some(mut current_minimap) = minimap() else {
                            continue;
                        };
                        current_minimap.paths_id_index = paths_id_index;
                        if let Some(current_minimap) = upsert_minimap(current_minimap).await {
                            minimap.set(Some(current_minimap));
                        }
                    }
                }
            }
        },
    );
    let on_add_point = use_callback::<(NavigationPath, usize), _>(move |(path, path_index)| {
        popup.set(Some(NavigationPopup::Point(
            path,
            path_index,
            PopupPointValue::Add(NavigationPoint {
                next_paths_id_index: None,
                x: position.peek().0,
                y: position.peek().1,
                transition: NavigationTransition::Portal,
            }),
        )));
    });
    let on_delete_point = use_callback::<(NavigationPath, usize, usize), _>(
        move |(mut path, path_index, point_index)| {
            let Some(mut paths) = selected_paths.peek().clone() else {
                return;
            };
            if path.points.get(point_index).is_some() {
                path.points.remove(point_index);
            }
            if let Some(path_mut) = paths.paths.get_mut(path_index) {
                *path_mut = path;
                coroutine.send(NavigationUpdate::Update(paths));
            }
        },
    );
    let on_create_path = use_callback(move |_| async move {
        let Some(mut paths) = selected_paths() else {
            return;
        };
        let Some(path) = create_navigation_path().await else {
            return;
        };

        paths.paths.push(path);
        coroutine.send(NavigationUpdate::Update(paths));
    });
    let on_delete_path = use_callback::<usize, _>(move |path_index| {
        let Some(mut paths) = selected_paths.peek().clone() else {
            return;
        };
        paths.paths.remove(path_index);
        coroutine.send(NavigationUpdate::Update(paths));
    });
    let on_select_paths = use_callback::<(NavigationPath, usize, usize, PathsIdIndex), _>(
        move |(mut path, path_index, point_index, next_paths_id_index)| {
            let Some(mut paths) = selected_paths.peek().clone() else {
                return;
            };
            if let Some(point) = path.points.get_mut(point_index) {
                point.next_paths_id_index = next_paths_id_index;
            }
            if let Some(path_mut) = paths.paths.get_mut(path_index) {
                *path_mut = path;
                coroutine.send(NavigationUpdate::Update(paths));
            }
        },
    );

    let on_popup_recapture = use_callback(move |(path, path_index)| async move {
        let Some(mut paths) = selected_paths() else {
            return;
        };
        let new_path = recapture_navigation_path(path).await;
        if let Some(path_mut) = paths.paths.get_mut(path_index) {
            *path_mut = new_path.clone();
        }
        popup.set(Some(NavigationPopup::Snapshots(new_path, path_index)));
        coroutine.send(NavigationUpdate::Update(paths));
    });
    let on_popup_point = use_callback::<(NavigationPath, usize, PopupPointValue), _>(
        move |(mut path, path_index, point_value)| {
            let Some(mut paths) = selected_paths.peek().clone() else {
                return;
            };
            match point_value {
                PopupPointValue::Add(point) => {
                    path.points.push(point);
                }
                PopupPointValue::Edit(new_point, index) => {
                    if let Some(point) = path.points.get_mut(index) {
                        *point = new_point;
                    }
                }
            }
            if let Some(path_mut) = paths.paths.get_mut(path_index) {
                *path_mut = path;
                coroutine.send(NavigationUpdate::Update(paths));
            }
        },
    );

    use_effect(move || {
        let paths = paths_view();
        if !paths.is_empty() && selected_paths.peek().is_none() {
            selected_paths.set(paths.into_iter().next());
        }
    });
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
                DatabaseEvent::NavigationPathsUpdated | DatabaseEvent::NavigationPathsDeleted
            ) {
                paths.restart();
            }
        }
    });

    rsx! {
        Section { name: "Selected map",
            div { class: "grid grid-cols-2 gap-3",
                Select {
                    label: "Attached paths group",
                    disabled: minimap().is_none(),
                    options: [vec!["None".to_string()], paths_names_view()].concat(),
                    on_select: move |(paths_index, _)| {
                        let next_paths_id = if paths_index == 0 {
                            None
                        } else {
                            let index = paths_index - 1;
                            let paths = paths_view.peek();
                            paths.get(index).and_then(|path: &NavigationPaths| path.id)
                        };
                        let next_paths_id_index = next_paths_id.map(|id| (id, 0));
                        coroutine.send(NavigationUpdate::Attach(next_paths_id_index));
                    },
                    selected: minimap_paths_index().unwrap_or_default(),
                }
                Select::<String> {
                    label: "Attached path",
                    placeholder: "None",
                    disabled: minimap_paths_index().is_none(),
                    options: minimap_paths_index_options(),
                    on_select: move |(path_index, _)| {
                        let Some((id, _)) = *minimap_paths_id_index.peek() else {
                            return;
                        };
                        coroutine.send(NavigationUpdate::Attach(Some((id, path_index))));
                    },
                    selected: minimap_paths_id_index().map(|(_, index)| index).unwrap_or_default(),
                }
            }
        }
        Section { name: "Paths",
            TextSelect {
                class: "w-full",
                options: paths_names_view(),
                disabled: false,
                placeholder: "Create a paths group...",
                on_create: move |name| {
                    coroutine.send(NavigationUpdate::Create(name));
                },
                on_delete: move |_| {
                    coroutine.send(NavigationUpdate::Delete);
                },
                on_select: move |(index, _)| {
                    let selected: NavigationPaths = paths_view.peek().get(index).cloned().unwrap();
                    selected_paths.set(Some(selected));
                },
                selected: selected_paths_index(),
            }
            if let Some(paths) = selected_paths() {
                div { class: "flex flex-col gap-3",
                    for (path_index , path) in paths.paths.into_iter().enumerate() {
                        NavigationPathItem {
                            path,
                            path_index,
                            paths_view,
                            paths_names_view,
                            on_add_point: move |path| {
                                on_add_point((path, path_index));
                            },
                            on_delete_point: move |(path, point_index)| {
                                on_delete_point((path, path_index, point_index));
                            },
                            on_edit_point: move |(path, point, index)| {
                                let edit = PopupPointValue::Edit(point, index);
                                let point = NavigationPopup::Point(path, path_index, edit);
                                popup.set(Some(point));
                            },
                            on_select_paths: move |(path, point_index, path_ids_index)| {
                                on_select_paths((path, path_index, point_index, path_ids_index));
                            },
                            on_delete_path: move |_| {
                                on_delete_path(path_index);
                            },
                            on_path_details: move |path: NavigationPath| {
                                popup.set(Some(NavigationPopup::Snapshots(path, path_index)));
                            },
                        }
                    }
                }
                Button {
                    text: "Add path",
                    kind: ButtonKind::Secondary,
                    on_click: move |_| async move {
                        on_create_path(()).await;
                    },
                    class: "label mt-4",
                }
            }
        }
        if let Some(kind) = popup() {
            match kind {
                NavigationPopup::Snapshots(path, path_index) => {
                    rsx! {
                        PopupSnapshots {
                            name_base64: path.name_snapshot_base64.clone(),
                            minimap_base64: path.minimap_snapshot_base64.clone(),
                            on_recapture: move |_| {
                                let path = path.clone();
                                async move {
                                    on_popup_recapture((path, path_index)).await;
                                }
                            },
                            on_cancel: move |_| {
                                popup.set(None);
                            },
                        }
                    }
                }
                NavigationPopup::Point(path, path_index, value) => {
                    rsx! {
                        PopupPoint {
                            value,
                            on_save: move |value| {
                                on_popup_point((path.clone(), path_index, value));
                                popup.set(None);
                            },
                            on_close: move |_| {
                                popup.set(None);
                            },
                        }
                    }
                }
            }
        }
    }
}

// TODO: Whether to give a cloned path in the callbacks or let caller clone. NavigationPath
//       does not implement Copy so it is kind of inconvenient right now.
#[component]
fn NavigationPathItem(
    path: NavigationPath,
    path_index: usize,
    paths_view: Memo<Vec<NavigationPaths>>,
    paths_names_view: Memo<Vec<String>>,
    on_add_point: EventHandler<NavigationPath>,
    on_edit_point: EventHandler<(NavigationPath, NavigationPoint, usize)>,
    on_delete_point: EventHandler<(NavigationPath, usize)>,
    on_select_paths: EventHandler<(NavigationPath, usize, PathsIdIndex)>,
    on_delete_path: EventHandler,
    on_path_details: EventHandler<NavigationPath>,
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
    let get_point_paths_index = use_callback(move |paths_id| {
        paths_view()
            .iter()
            .enumerate()
            .find_map(|(index, paths)| {
                if paths.id == Some(paths_id) {
                    Some(index + 1)
                } else {
                    None
                }
            })
            .unwrap_or_default()
    });
    let get_point_paths = use_callback(move |paths_id| {
        paths_view()
            .into_iter()
            .find_map(|paths| {
                if paths.id == Some(paths_id) {
                    Some(paths.paths)
                } else {
                    None
                }
            })
            .unwrap_or_default()
    });
    let get_point_path_options = use_callback(move |paths_id| {
        (0..get_point_paths(paths_id).len())
            .map(|index| format!("Path {}", index + 1))
            .collect::<Vec<String>>()
    });
    // For avoiding too long line
    let get_point_path_index = use_callback(move |paths_id_index: PathsIdIndex| {
        paths_id_index.map(|(_, index)| index).unwrap_or_default()
    });

    rsx! {
        div { class: "mt-3",
            div { class: "grid grid-cols-2 gap-x-3 group",
                div { class: "border-b border-gray-600 p-1",
                    img {
                        width: path().name_snapshot_width,
                        height: path().name_snapshot_height,
                        src: format!("data:image/png;base64,{}", path().name_snapshot_base64),
                    }
                }
                div { class: "grid grid-cols-3 gap-x-2 group",
                    p { class: "col-span-2 paragraph-xs flex items-center border-b border-gray-600",
                        {format!("Path {}", path_index + 1)}
                    }
                    Icons {
                        on_details: move |_| {
                            on_path_details(path.peek().clone());
                        },
                        on_delete: move |_| {
                            on_delete_path(());
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

                    div { class: "grid grid-cols-3 gap-x-2",
                        Select::<String> {
                            div_class: "!gap-0",
                            options: [vec!["None".to_string()], paths_names_view()].concat(),
                            on_select: move |(paths_index, _)| {
                                let next_paths_id = if paths_index == 0 {
                                    None
                                } else {
                                    let index = paths_index - 1;
                                    let paths = paths_view.peek();
                                    paths.get(index).and_then(|path: &NavigationPaths| path.id)
                                };
                                let next_paths_id_index = next_paths_id.map(|id| (id, 0));
                                on_select_paths((path.peek().clone(), index, next_paths_id_index));
                            },
                            selected: point
                                .next_paths_id_index
                                .map(|(id, _)| get_point_paths_index(id))
                                .unwrap_or_default(),
                        }
                        Select::<String> {
                            div_class: "!gap-0",
                            placeholder: "None",
                            options: point
                                .next_paths_id_index
                                .map(|(id, _)| get_point_path_options(id))
                                .unwrap_or_default(),
                            on_select: move |(path_index, _)| {
                                let next_paths_id_index = point
                                    .next_paths_id_index
                                    .map(|(id, _)| (id, path_index));
                                on_select_paths((path.peek().clone(), index, next_paths_id_index));
                            },
                            selected: get_point_path_index(point.next_paths_id_index),
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
