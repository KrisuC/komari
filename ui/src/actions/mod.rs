use std::{collections::HashMap, fmt::Display, mem::discriminant};

use backend::{
    Action, ActionCondition, IntoEnumIterator, KeyBinding, Map, Settings, update_map, upsert_map,
};
use dioxus::prelude::*;
use futures_util::StreamExt;
use inner::SectionActions;
use platforms::SectionPlatforms;
use rotation::SectionRotation;

use crate::{
    AppState, persist_settings,
    components::{
        ContentAlign, ContentSide,
        checkbox::Checkbox,
        key::KeyInput,
        labeled::Labeled,
        named_select::NamedSelect,
        numbers::{MillisInput, PrimitiveIntegerInput},
        position::PositionInput,
        section::Section,
        select::{Select, SelectOption},
    },
};

mod inner;
mod input;
mod list;
mod platforms;
mod popup;
mod rotation;

const ITEM_TEXT_CLASS: &str =
    "text-center inline-block pt-1 text-ellipsis overflow-hidden whitespace-nowrap";
const ITEM_BORDER_CLASS: &str = "border-r-2 border-secondary-border";

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum ActionsUpdate {
    Set,
    Create(String),
    Delete,
    Update(Vec<Action>),
    UpdateMap(Map),
}

#[derive(PartialEq, Copy, Clone)]
struct ActionsContext {
    map: Memo<Map>,
    save_map: Callback<Map>,
    lists: Signal<HashMap<String, ActionCondition>>,
}

#[component]
pub fn ActionsScreen() -> Element {
    let mut map = use_context::<AppState>().map;
    let mut map_preset = use_context::<AppState>().map_preset;
    let settings = use_context::<AppState>().settings;
    // Non-null view of map
    let map_view = use_memo(move || map().unwrap_or_default());
    // Maps currently selected `map` to presets
    let map_presets = use_memo(move || {
        map()
            .map(|map| map.actions.into_keys().collect::<Vec<String>>())
            .unwrap_or_default()
    });
    // Maps currently selected `map_preset` to actions
    let map_preset_actions = use_memo(move || {
        map()
            .zip(map_preset())
            .and_then(|(map, preset)| map.actions.get(&preset).cloned())
            .unwrap_or_default()
    });
    // Maps currently selected `map_preset` to the index in `map_presets`
    let map_preset_index = use_memo(move || {
        let presets = map_presets();
        map_preset().and_then(|preset| {
            presets
                .into_iter()
                .enumerate()
                .find(|(_, p)| &preset == p)
                .map(|(i, _)| i)
        })
    });

    // Handles async operations for action-related
    let coroutine = use_coroutine(move |mut rx: UnboundedReceiver<ActionsUpdate>| async move {
        while let Some(message) = rx.next().await {
            match message {
                ActionsUpdate::Set => {
                    update_map(map(), map_preset()).await;
                }
                ActionsUpdate::Create(preset) => {
                    let Some(mut current_map) = map() else {
                        continue;
                    };
                    if current_map
                        .actions
                        .try_insert(preset.clone(), vec![])
                        .is_err()
                    {
                        continue;
                    }
                    if let Some(current_map) = upsert_map(current_map).await {
                        map_preset.set(Some(preset));
                        map.set(Some(current_map));
                        update_map(map(), map_preset()).await;
                    }
                }
                ActionsUpdate::Delete => {
                    let Some(mut current_map) = map() else {
                        continue;
                    };
                    let Some(preset) = map_preset() else {
                        continue;
                    };

                    if current_map.actions.remove(&preset).is_none() {
                        continue;
                    }
                    if let Some(current_map) = upsert_map(current_map).await {
                        map_preset.set(current_map.actions.keys().next().cloned());
                        map.set(Some(current_map));
                        update_map(map(), map_preset()).await;
                    }
                }
                ActionsUpdate::Update(actions) => {
                    let Some(mut current_map) = map() else {
                        continue;
                    };
                    let Some(preset) = map_preset() else {
                        continue;
                    };

                    current_map.actions.insert(preset, actions);
                    if let Some(current_map) = upsert_map(current_map).await {
                        map.set(Some(current_map));
                        update_map(map(), map_preset()).await;
                    }
                }
                ActionsUpdate::UpdateMap(new_map) => {
                    if let Some(new_map) = upsert_map(new_map).await {
                        map.set(Some(new_map));
                        update_map(map(), map_preset()).await;
                    }
                }
            }
        }
    });

    let save_map = use_callback(move |map: Map| {
        coroutine.send(ActionsUpdate::UpdateMap(map));
    });
    let select_preset = use_callback(move |index: usize| {
        let selected = map_presets.peek().get(index).cloned().unwrap();

        map_preset.set(Some(selected.clone()));
        coroutine.send(ActionsUpdate::Set);
        persist_settings(settings, move |current| Settings {
            selected_map_preset: Some(selected),
            ..current
        });
    });

    let lists = use_signal::<HashMap<String, ActionCondition>>(HashMap::default);
    use_context_provider(|| ActionsContext {
        map: map_view,
        save_map,
        lists,
    });

    rsx! {
        div { class: "flex flex-col pb-15 h-full gap-3 overflow-y-auto pr-2",
            SectionRotation { disabled: map().is_none() }
            SectionPlatforms { disabled: map().is_none() }
            SectionActions {
                actions: map_preset_actions,
                disabled: map().is_none() || map_preset().is_none(),
            }
            SectionLegends {}
        }

        div { class: "flex items-center w-full h-10 pr-2 bg-primary-surface absolute bottom-0",
            NamedSelect {
                class: "flex-grow",
                on_create: move |name| {
                    coroutine.send(ActionsUpdate::Create(name));
                },
                on_delete: move |_| {
                    coroutine.send(ActionsUpdate::Delete);
                },
                disabled: map().is_none(),
                delete_disabled: map_presets().is_empty(),

                Select::<usize> {
                    class: "w-full",
                    placeholder: "Create an actions preset for the selected map...",
                    disabled: map_presets().is_empty(),
                    on_selected: select_preset,

                    for (i , name) in map_presets().into_iter().enumerate() {
                        SelectOption::<usize> {
                            value: i,
                            selected: map_preset_index() == Some(i),
                            label: name,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SectionLegends() -> Element {
    rsx! {
        Section { title: "Action legends", class: "text-xs text-primary-text",
            p { "⟳ - Repeat" }
            p { "⏱︎  - Wait" }
            p { "ㄨ - No position" }
            p { "⇈ - Queue to front" }
            p { "⇆ - Any direction" }
            p { "← - Left direction" }
            p { "→ - Right direction" }
            p { "⁺ - Buffered wait after" }
            p { "A ⤓ - Key A is held down" }
            p { "A ~ B - Random range between A and B" }
            p { "A ↝ B - Use A key then B key" }
            p { "A ↜ B - Use B key then A key" }
            p { "A ↭ B - Use A and B keys at the same time" }
            p { "A ↷ B - Use A key then B key while A is held down" }
        }
    }
}

#[component]
fn ActionsSelect<T: 'static + Clone + PartialEq + Display + IntoEnumIterator>(
    label: &'static str,
    #[props(default)] tooltip: Option<String>,
    #[props(default = ContentAlign::Start)] tooltip_align: ContentAlign,
    disabled: bool,
    on_selected: Callback<T>,
    selected: ReadSignal<T>,
) -> Element {
    let selected_equal =
        use_callback(move |value: T| discriminant(&selected()) == discriminant(&value));

    rsx! {
        Labeled { label, tooltip, tooltip_align,
            Select::<T> { on_selected, disabled,

                for value in T::iter() {
                    SelectOption::<T> {
                        value: value.clone(),
                        label: value.to_string(),
                        selected: selected_equal(value),
                        disabled,
                    }
                }
            }
        }
    }
}

#[component]
fn ActionsPositionInput(
    label: &'static str,
    #[props(default)] disabled: bool,
    on_icon_click: ReadSignal<Option<Callback>>,
    on_value: Callback<i32>,
    value: i32,
) -> Element {
    rsx! {
        Labeled { label,
            PositionInput {
                disabled,
                on_icon_click,
                on_value,
                value,
            }
        }
    }
}

#[component]
fn ActionsNumberInputI32(
    label: &'static str,
    #[props(default)] disabled: bool,
    on_value: Callback<i32>,
    value: i32,
) -> Element {
    rsx! {
        Labeled { label,
            PrimitiveIntegerInput { disabled, on_value, value }
        }
    }
}

#[component]
fn ActionsNumberInputU32(
    label: &'static str,
    #[props(default)] disabled: bool,
    on_value: Callback<u32>,
    value: u32,
) -> Element {
    rsx! {
        Labeled { label,
            PrimitiveIntegerInput {
                disabled,
                on_value,
                value,
                min_value: 1,
            }
        }
    }
}

#[component]
fn ActionsMillisInput(
    label: &'static str,
    #[props(default)] disabled: bool,
    on_value: Callback<u64>,
    value: u64,
) -> Element {
    rsx! {
        Labeled { label,
            MillisInput { disabled, on_value, value }
        }
    }
}

#[component]
fn ActionsCheckbox(
    label: &'static str,
    #[props(default)] tooltip: Option<String>,
    #[props(default = ContentSide::Left)] tooltip_side: ContentSide,
    #[props(default = ContentAlign::End)] tooltip_align: ContentAlign,
    #[props(default)] disabled: bool,
    on_checked: Callback<bool>,
    checked: bool,
) -> Element {
    rsx! {
        Labeled {
            label,
            tooltip,
            tooltip_side,
            tooltip_align,
            Checkbox { disabled, on_checked, checked }
        }
    }
}

#[component]
fn ActionsKeyBindingInput(
    label: &'static str,
    disabled: bool,
    on_value: Callback<Option<KeyBinding>>,
    value: Option<KeyBinding>,
) -> Element {
    rsx! {
        Labeled { label,
            KeyInput {
                class: "border border-primary-border",
                disabled,
                on_value: move |value: Option<KeyBinding>| {
                    on_value(value);
                },
                value,
            }
        }
    }
}
