use std::{fmt::Display, mem::discriminant};

use backend::IntoEnumIterator;
use dioxus::prelude::*;

use crate::{
    button::{Button, ButtonKind},
    inputs::LabeledInput,
};

// Pre-styled
const INPUT_LABEL_CLASS: &str = "label";
const INPUT_DIV_CLASS: &str = "flex flex-col gap-1";
const INPUT_SELECT_CLASS: &str = "items-center picker:scroll-bar paragraph-xs outline-none px-1 border border-gray-600 disabled:text-gray-600 disabled:cursor-not-allowed whitespace-nowrap text-ellipsis";
const INPUT_OPTION_CLASS: &str = "bg-gray-900 paragraph-xs pl-1 pr-2 hover:bg-gray-800";

#[derive(PartialEq, Props, Clone)]
pub struct SelectProps<T: 'static + Clone + PartialEq + Display> {
    #[props(default = String::default())]
    label: String,
    #[props(default = String::default())]
    label_class: String,
    #[props(default = String::default())]
    div_class: String,
    #[props(default = String::default())]
    select_class: String,
    #[props(default = String::default())]
    option_class: String,
    #[props(default = false)]
    disabled: bool,
    #[props(default = String::default())]
    placeholder: String,
    options: Vec<T>,
    on_select: EventHandler<(usize, T)>,
    selected: usize,
}

// TODO: Please https://github.com/DioxusLabs/dioxus/issues/3938
#[component]
pub fn EnumSelect<T: 'static + Clone + PartialEq + Display + IntoEnumIterator>(
    label: String,
    #[props(default = String::default())] label_class: String,
    #[props(default = String::default())] div_class: String,
    #[props(default = String::default())] select_class: String,
    #[props(default = String::default())] option_class: String,
    #[props(default = false)] disabled: bool,
    on_select: EventHandler<T>,
    selected: T,
    #[props(default = Vec::new())] excludes: Vec<T>,
) -> Element {
    let options = T::iter()
        .filter(|variant| !excludes.contains(variant))
        .collect::<Vec<_>>();
    let selected = options
        .iter()
        .enumerate()
        .find(|(_, option)| discriminant(&selected) == discriminant(option))
        .map(|(i, _)| i)
        .unwrap_or_default();

    rsx! {
        Select {
            label,
            disabled,
            div_class,
            label_class,
            select_class,
            option_class,
            options,
            on_select: move |(_, variant): (usize, T)| {
                on_select(variant);
            },
            selected,
        }
    }
}

#[component]
pub fn TextSelect(
    class: String,
    options: Vec<String>,
    disabled: bool,
    #[props(default = String::default())] placeholder: String,
    on_create: EventHandler<String>,
    on_delete: EventHandler<usize>,
    on_select: EventHandler<(usize, String)>,
    selected: Option<usize>,
) -> Element {
    let mut creating_text = use_signal::<Option<String>>(|| None);
    let mut creating_error = use_signal(|| false);
    let reset_creating = use_callback(move |_| {
        creating_text.set(None);
        creating_error.set(false);
    });
    let select_or_delete_disabled = disabled || options.is_empty();

    use_effect(use_reactive!(|selected| {
        if selected.is_none() {
            reset_creating(());
        }
    }));
    use_effect(use_reactive!(|disabled| {
        if disabled {
            reset_creating(());
        }
    }));

    rsx! {
        div { class: "flex gap-3 h-6 {class}",
            div { class: "flex-grow",
                if let Some(text) = creating_text() {
                    div { class: "relative h-full",
                        input {
                            class: "absolute inset-0 w-full h-full px-1 border border-gray-600 paragraph-xs outline-none",
                            placeholder: "Enter a name...",
                            onchange: move |e| {
                                creating_text.set(Some(e.value()));
                            },
                            value: text,
                        }
                    }
                } else {
                    Select {
                        div_class: "relative h-full",
                        select_class: "absolute inset-0 px-1 w-full h-full border border-gray-600 paragraph-xs outline-none items-center",
                        option_class: "paragraph-xs bg-gray-900 px-2 hover:bg-gray-800",
                        disabled: select_or_delete_disabled,
                        placeholder,
                        options,
                        on_select: move |(usize, text)| {
                            on_select((usize, text));
                        },
                        selected: selected.unwrap_or_default(),
                    }
                }
            }
            Button {
                class: "w-20",
                text: if creating_text().is_some() { "Save" } else { "Create" },
                kind: ButtonKind::Primary,
                disabled,
                on_click: move |_| {
                    let text = creating_text.peek().clone();
                    if let Some(text) = text {
                        if text.is_empty() {
                            creating_error.set(true);
                            return;
                        }
                        reset_creating(());
                        on_create(text);
                    } else {
                        creating_text.set(Some("".to_string()));
                    }
                },
            }
            Button {
                class: "w-20",
                text: if creating_text().is_some() { "Cancel" } else { "Delete" },
                kind: ButtonKind::Danger,
                disabled: select_or_delete_disabled && creating_text().is_none(),
                on_click: move |_| {
                    if creating_text.peek().is_some() {
                        reset_creating(());
                    } else if let Some(index) = selected {
                        on_delete(index);
                    }
                },
            }
        }
    }
}

#[component]
pub fn Select<T>(
    SelectProps {
        label,
        div_class,
        label_class,
        select_class,
        option_class,
        options,
        disabled,
        placeholder,
        on_select,
        selected,
    }: SelectProps<T>,
) -> Element
where
    T: 'static + Clone + PartialEq + Display,
{
    let option_class = format!("{INPUT_OPTION_CLASS} {option_class}");

    rsx! {
        LabeledInput {
            label,
            label_class: "{INPUT_LABEL_CLASS} {label_class}",
            div_class: "{INPUT_DIV_CLASS} {div_class}",
            disabled,
            select {
                class: "{INPUT_SELECT_CLASS} {select_class}",
                disabled,
                onchange: move |e| {
                    let i = e.value().parse::<usize>().unwrap();
                    let value = options[i].clone();
                    on_select((i, value))
                },
                if options.is_empty() {
                    option {
                        class: option_class.clone(),
                        disabled: true,
                        selected: true,
                        value: "",
                        label: placeholder,
                    }
                }
                for (i , option) in options.iter().enumerate() {
                    option {
                        class: option_class.clone(),
                        disabled,
                        selected: i == selected,
                        value: i.to_string(),
                        label: option.to_string(),
                    }
                }
            }
        }
    }
}
