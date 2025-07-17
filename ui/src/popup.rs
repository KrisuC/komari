use dioxus::prelude::*;

use crate::button::{Button, ButtonKind};

#[component]
pub fn Popup(
    title: String,
    #[props(default = String::default())] class: String,
    confirm_button: Option<String>,
    on_confirm: Option<EventHandler>,
    cancel_button: Option<String>,
    on_cancel: Option<EventHandler>,
    children: Element,
) -> Element {
    let confirm = confirm_button.zip(on_confirm);
    let cancel = cancel_button.zip(on_cancel);
    let bottom_pad = if confirm.is_some() || cancel.is_some() {
        "pb-10"
    } else {
        ""
    };

    rsx! {
        div { class: "absolute inset-0 z-1 bg-gray-950/80 flex",
            div { class: "bg-gray-900 px-2 w-full h-full {class} m-auto",
                div { class: "flex flex-col h-full gap-2 relative {bottom_pad}",
                    div { class: "flex flex-none items-center title-xs h-10", {title} }
                    {children}
                    if confirm.is_some() || cancel.is_some() {
                        div { class: "flex w-full gap-3 absolute bottom-0 py-2 bg-gray-900",
                            if let Some((confirm_button, on_confirm)) = confirm {
                                Button {
                                    class: "flex-grow border border-gray-600",
                                    text: confirm_button,
                                    kind: ButtonKind::Secondary,
                                    on_click: move |_| {
                                        on_confirm(());
                                    },
                                }
                            }
                            if let Some((cancel_button, on_cancel)) = cancel {
                                Button {
                                    class: "flex-grow border border-gray-600",
                                    text: cancel_button,
                                    kind: ButtonKind::Secondary,
                                    on_click: move |_| {
                                        on_cancel(());
                                    },
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
