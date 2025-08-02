use dioxus::prelude::*;

use super::{INPUT_CLASS, INPUT_DIV_CLASS, INPUT_LABEL_CLASS, LabeledInput};

#[derive(Clone, PartialEq, Props)]
pub struct TextInputProps {
    label: String,
    #[props(default = String::default())]
    label_class: String,
    #[props(default = String::default())]
    div_class: String,
    #[props(default = String::default())]
    input_class: String,
    #[props(default = false)]
    disabled: bool,
    #[props(default = false)]
    hidden: bool,
    on_value: EventHandler<String>,
    value: String,
}

#[component]
pub fn TextInput(
    TextInputProps {
        label,
        label_class,
        div_class,
        input_class,
        disabled,
        hidden,
        on_value,
        value,
    }: TextInputProps,
) -> Element {
    rsx! {
        LabeledInput {
            label,
            label_class: "{INPUT_LABEL_CLASS} {label_class}",
            disabled,
            div_class: "{INPUT_DIV_CLASS} {div_class}",
            div { class: "{INPUT_CLASS} {input_class}",
                input {
                    class: "outline-none disabled:cursor-not-allowed w-full h-full",
                    disabled,
                    r#type: if hidden { "password" } else { "text" },
                    oninput: move |e| {
                        on_value(e.parsed::<String>().unwrap());
                    },
                    value,
                }
            }
        }
    }
}
