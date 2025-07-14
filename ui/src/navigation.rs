use backend::{
    NavigationPath, NavigationPoint, NavigationTransition, create_navigation_path,
    delete_navigation_path, query_navigation_paths, upsert_navigation_path,
};
use dioxus::prelude::*;
use futures_util::StreamExt;

use crate::{
    AppState,
    button::{Button, ButtonKind},
    icons::XIcon,
    select::Select,
};

#[derive(Debug)]
enum NavigationUpdate {
    Update(NavigationPath),
    Create,
    Delete(NavigationPath),
}

#[component]
pub fn Navigation() -> Element {
    rsx! {
        div { class: "flex flex-col h-full overflow-y-auto scrollbar", SectionPaths {} }
    }
}

#[component]
fn SectionPaths() -> Element {
    let mut paths = use_resource(async || query_navigation_paths().await.unwrap_or_default());
    let paths_view = use_memo(move || paths().unwrap_or_default());
    let position = use_context::<AppState>().position;

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
            div { class: "flex flex-col gap-4",
                for path in paths_view() {
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
) -> Element {
    #[component]
    fn Icons(on_delete: EventHandler) -> Element {
        const ICON_CONTAINER_CLASS: &str = "w-4 h-6 flex justify-center items-center";
        const ICON_CLASS: &str = "w-[11px] h-[11px] fill-current";

        rsx! {
            div { class: "invisible group-hover:visible flex",
                div { class: "flex-grow" }
                div {
                    class: ICON_CONTAINER_CLASS,
                    onclick: move |e| {
                        e.stop_propagation();
                        on_delete(());
                    },
                    XIcon { class: "{ICON_CLASS} text-red-500" }
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
fn Section(name: &'static str, children: Element) -> Element {
    rsx! {
        div { class: "flex flex-col pr-4 pb-3",
            div { class: "flex items-center title-xs h-10", {name} }
            {children}
        }
    }
}
