use dioxus::prelude::*;

mod keys;
mod numbers;
mod text;

pub use {keys::*, numbers::*, text::*};

// Pre-styled
pub(crate) const INPUT_LABEL_CLASS: &str =
    "label inline-block whitespace-nowrap overflow-hidden text-ellipsis";
pub(crate) const INPUT_DIV_CLASS: &str = "flex flex-col gap-1";
pub(crate) const INPUT_CLASS: &str = "h-6 paragraph-xs outline-none px-1 border border-gray-600 disabled:text-gray-600 disabled:cursor-not-allowed";

#[derive(Clone, PartialEq, Props)]
pub struct GenericInputProps<T: 'static + Clone + PartialEq> {
    label: String,
    #[props(default = String::default())]
    label_class: String,
    #[props(default = String::default())]
    div_class: String,
    #[props(default = String::default())]
    input_class: String,
    #[props(default = false)]
    disabled: bool,
    on_value: EventHandler<T>,
    value: T,
}

#[component]
pub fn Checkbox(
    GenericInputProps {
        label,
        label_class,
        div_class,
        input_class,
        disabled,
        on_value,
        value,
    }: GenericInputProps<bool>,
) -> Element {
    rsx! {
        LabeledInput {
            label,
            label_class: "{INPUT_LABEL_CLASS} {label_class}",
            div_class: "{INPUT_DIV_CLASS} {div_class}",
            disabled,
            div { class: "{INPUT_CLASS} {input_class}",
                input {
                    class: "appearance-none w-full h-full disabled:cursor-not-allowed",
                    disabled,
                    r#type: "checkbox",
                    oninput: move |e| {
                        on_value(e.parsed::<bool>().unwrap());
                    },
                    checked: value,
                }
            }
        }
    }
}

#[derive(Clone, PartialEq, Props)]
pub(crate) struct LabeledInputProps {
    label: String,
    label_class: String,
    div_class: String,
    disabled: bool,
    children: Element,
}

#[component]
pub(crate) fn LabeledInput(props: LabeledInputProps) -> Element {
    let data_disabled = props.disabled.then_some(true);

    rsx! {
        div { class: props.div_class, "data-disabled": data_disabled,
            label { class: props.label_class, "data-disabled": data_disabled, {props.label} }
            {props.children}
        }
    }
}
