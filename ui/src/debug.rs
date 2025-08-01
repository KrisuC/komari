use backend::{
    DebugState, auto_save_rune, capture_image, debug_state_receiver, infer_minimap, infer_rune,
    record_images, test_spin_rune,
};
use dioxus::prelude::*;
use tokio::sync::broadcast::error::RecvError;

use crate::button::{Button, ButtonKind};

#[component]
pub fn Debug() -> Element {
    let mut state = use_signal(DebugState::default);

    use_future(move || async move {
        let mut rx = debug_state_receiver().await;
        loop {
            let current_state = match rx.recv().await {
                Ok(state) => state,
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            };
            if current_state != *state.peek() {
                state.set(current_state);
            }
        }
    });

    rsx! {
        div { class: "flex flex-col h-full overflow-y-auto scrollbar",
            Section { name: "Debug",
                div { class: "grid grid-cols-2 gap-3",
                    Button {
                        text: "Capture color image",
                        kind: ButtonKind::Secondary,
                        on_click: move |_| async {
                            capture_image(false).await;
                        },
                    }
                    Button {
                        text: "Capture grayscale image",
                        kind: ButtonKind::Secondary,
                        on_click: move |_| async {
                            capture_image(true).await;
                        },
                    }
                    Button {
                        text: "Infer rune",
                        kind: ButtonKind::Secondary,
                        on_click: move |_| async {
                            infer_rune().await;
                        },
                    }
                    Button {
                        text: "Infer minimap",
                        kind: ButtonKind::Secondary,
                        on_click: move |_| async {
                            infer_minimap().await;
                        },
                    }
                    Button {
                        text: "Spin rune sandbox test",
                        kind: ButtonKind::Secondary,
                        on_click: move |_| async {
                            test_spin_rune().await;
                        },
                    }
                    Button {
                        text: if state().is_recording { "Stop recording" } else { "Start recording" },
                        kind: ButtonKind::Secondary,
                        on_click: move |_| async move {
                            record_images(!state.peek().is_recording).await;
                        },
                    }
                    Button {
                        text: if state().is_rune_auto_saving { "Stop auto saving rune" } else { "Start auto saving rune" },
                        kind: ButtonKind::Secondary,
                        on_click: move |_| async move {
                            auto_save_rune(!state.peek().is_rune_auto_saving).await;
                        },
                    }
                }
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
