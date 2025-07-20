use std::{fmt::Display, fs::File, io::BufReader};

use backend::{
    CaptureMode, FamiliarRarity, Familiars, InputMethod, IntoEnumIterator, KeyBinding,
    KeyBindingConfiguration, Notifications, Settings as SettingsData, SwappableFamiliars,
    query_capture_handles, query_settings, select_capture_handle, upsert_settings,
};
use dioxus::prelude::*;
use futures_util::StreamExt;
use rand::distr::{Alphanumeric, SampleString};

use crate::{
    AppState,
    button::{Button, ButtonKind},
    inputs::{Checkbox, KeyBindingInput, MillisInput, TextInput},
    select::{EnumSelect, Select},
};

#[derive(Debug)]
enum SettingsUpdate {
    Update(SettingsData),
}

#[component]
pub fn Settings() -> Element {
    let mut settings = use_context::<AppState>().settings;
    let settings_view = use_memo(move || settings().unwrap_or_default());

    // Handles async operations for settings-related
    let coroutine = use_coroutine(
        move |mut rx: UnboundedReceiver<SettingsUpdate>| async move {
            while let Some(message) = rx.next().await {
                match message {
                    SettingsUpdate::Update(new_settings) => {
                        settings.set(Some(upsert_settings(new_settings).await));
                    }
                }
            }
        },
    );
    let save_settings = use_callback(move |new_settings: SettingsData| {
        coroutine.send(SettingsUpdate::Update(new_settings));
    });

    use_future(move || async move {
        if settings.peek().is_none() {
            settings.set(Some(query_settings().await));
        }
    });

    rsx! {
        div { class: "flex flex-col h-full overflow-y-auto scrollbar",
            SectionCapture { settings_view, save_settings }
            SectionInput { settings_view, save_settings }
            SectionFamiliars { settings_view, save_settings }
            SectionNotifications { settings_view, save_settings }
            SectionHotkeys { settings_view, save_settings }
            SectionRunStopCycle { settings_view, save_settings }
            SectionOthers { settings_view, save_settings }
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

#[component]
fn SectionCapture(
    settings_view: Memo<SettingsData>,
    save_settings: EventHandler<SettingsData>,
) -> Element {
    let mut selected_handle_index = use_signal(|| None);
    let mut handle_names = use_resource(move || async move {
        let (names, selected) = query_capture_handles().await;
        selected_handle_index.set(selected);
        names
    });
    let handle_names_with_default = use_memo(move || {
        let default = vec!["Default".to_string()];
        let names = handle_names().unwrap_or_default();

        [default, names].concat()
    });

    rsx! {
        Section { name: "Capture",
            div { class: "grid grid-cols-2 gap-3",
                SettingsSelect {
                    label: "Handle",
                    options: handle_names_with_default(),
                    on_select: move |(index, _)| async move {
                        if index == 0 {
                            selected_handle_index.set(None);
                            select_capture_handle(None).await;
                        } else {
                            selected_handle_index.set(Some(index - 1));
                            select_capture_handle(Some(index - 1)).await;
                        }
                    },
                    selected: selected_handle_index().map(|index| index + 1).unwrap_or_default(),
                }
                SettingsEnumSelect::<CaptureMode> {
                    label: "Mode",
                    on_select: move |capture_mode| {
                        save_settings(SettingsData {
                            capture_mode,
                            ..settings_view.peek().clone()
                        });
                    },
                    selected: settings_view().capture_mode,
                }
            }
            Button {
                text: "Refresh handles",
                kind: ButtonKind::Secondary,
                on_click: move |_| {
                    handle_names.restart();
                },
                class: "mt-2",
            }
        }
    }
}

#[component]
fn SectionInput(
    settings_view: Memo<SettingsData>,
    save_settings: EventHandler<SettingsData>,
) -> Element {
    rsx! {
        Section { name: "Input",
            div { class: "grid grid-cols-3 gap-3",
                SettingsEnumSelect::<InputMethod> {
                    label: "Method",
                    on_select: move |input_method| async move {
                        save_settings(SettingsData {
                            input_method,
                            ..settings_view.peek().clone()
                        });
                    },
                    selected: settings_view().input_method,
                }
                SettingsTextInput {
                    text_label: "RPC server URL",
                    button_label: "Update",
                    on_value: move |input_method_rpc_server_url| {
                        save_settings(SettingsData {
                            input_method_rpc_server_url,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().input_method_rpc_server_url,
                }
            }
        }
    }
}

#[component]
fn SectionFamiliars(
    settings_view: Memo<SettingsData>,
    save_settings: EventHandler<SettingsData>,
) -> Element {
    let familiars_view = use_memo(move || settings_view().familiars);

    rsx! {
        Section { name: "Familiars",
            SettingsCheckbox {
                label: "Enable swapping",
                on_value: move |enable_familiars_swapping| {
                    save_settings(SettingsData {
                        familiars: Familiars {
                            enable_familiars_swapping,
                            ..familiars_view.peek().clone()
                        },
                        ..settings_view.peek().clone()
                    });
                },
                value: familiars_view().enable_familiars_swapping,
            }
            div { class: "grid grid-cols-2 gap-3 mt-2",
                SettingsEnumSelect::<SwappableFamiliars> {
                    label: "Swappable slots",
                    disabled: !familiars_view().enable_familiars_swapping,
                    on_select: move |swappable_familiars| async move {
                        save_settings(SettingsData {
                            familiars: Familiars {
                                swappable_familiars,
                                ..familiars_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    selected: familiars_view().swappable_familiars,
                }
                MillisInput {
                    label: "Swap check every",
                    disabled: !familiars_view().enable_familiars_swapping,
                    on_value: move |swap_check_millis| {
                        save_settings(SettingsData {
                            familiars: Familiars {
                                swap_check_millis,
                                ..familiars_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: familiars_view().swap_check_millis,
                }

                SettingsCheckbox {
                    label: "Can swap rare familiars",
                    disabled: !familiars_view().enable_familiars_swapping,
                    on_value: move |allowed| {
                        let mut rarities = familiars_view.peek().swappable_rarities.clone();
                        if allowed {
                            rarities.insert(FamiliarRarity::Rare);
                        } else {
                            rarities.remove(&FamiliarRarity::Rare);
                        }
                        save_settings(SettingsData {
                            familiars: Familiars {
                                swappable_rarities: rarities,
                                ..familiars_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: familiars_view().swappable_rarities.contains(&FamiliarRarity::Rare),
                }
                SettingsCheckbox {
                    label: "Can swap epic familiars",
                    disabled: !familiars_view().enable_familiars_swapping,
                    on_value: move |allowed| {
                        let mut rarities = familiars_view.peek().swappable_rarities.clone();
                        if allowed {
                            rarities.insert(FamiliarRarity::Epic);
                        } else {
                            rarities.remove(&FamiliarRarity::Epic);
                        }
                        save_settings(SettingsData {
                            familiars: Familiars {
                                swappable_rarities: rarities,
                                ..familiars_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: familiars_view().swappable_rarities.contains(&FamiliarRarity::Epic),
                }
            }
        }
    }
}

#[component]
fn SectionNotifications(
    settings_view: Memo<SettingsData>,
    save_settings: EventHandler<SettingsData>,
) -> Element {
    let notifications_view = use_memo(move || settings_view().notifications);

    rsx! {
        Section { name: "Notifications",
            div { class: "grid grid-cols-2 gap-3 mb-2",
                SettingsTextInput {
                    text_label: "Discord webhook URL",
                    button_label: "Update",
                    on_value: move |discord_webhook_url| {
                        save_settings(SettingsData {
                            notifications: Notifications {
                                discord_webhook_url,
                                ..notifications_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: notifications_view().discord_webhook_url,
                }
                SettingsTextInput {
                    text_label: "Discord ping user ID",
                    button_label: "Update",
                    on_value: move |discord_user_id| {
                        save_settings(SettingsData {
                            notifications: Notifications {
                                discord_user_id,
                                ..notifications_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: notifications_view().discord_user_id,
                }
            }
            div { class: "grid grid-cols-3 gap-3",
                SettingsCheckbox {
                    label: "Rune spawns",
                    on_value: move |notify_on_rune_appear| {
                        save_settings(SettingsData {
                            notifications: Notifications {
                                notify_on_rune_appear,
                                ..notifications_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: notifications_view().notify_on_rune_appear,
                }
                SettingsCheckbox {
                    label: "Elite boss spawns",
                    on_value: move |notify_on_elite_boss_appear| {
                        save_settings(SettingsData {
                            notifications: Notifications {
                                notify_on_elite_boss_appear,
                                ..notifications_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: notifications_view().notify_on_elite_boss_appear,
                }
                SettingsCheckbox {
                    label: "Player dies",
                    on_value: move |notify_on_player_die| {
                        save_settings(SettingsData {
                            notifications: Notifications {
                                notify_on_player_die,
                                ..notifications_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: notifications_view().notify_on_player_die,
                }
                SettingsCheckbox {
                    label: "Guildie appears",
                    on_value: move |notify_on_player_guildie_appear| {
                        save_settings(SettingsData {
                            notifications: Notifications {
                                notify_on_player_guildie_appear,
                                ..notifications_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: notifications_view().notify_on_player_guildie_appear,
                }
                SettingsCheckbox {
                    label: "Stranger appears",
                    on_value: move |notify_on_player_stranger_appear| {
                        save_settings(SettingsData {
                            notifications: Notifications {
                                notify_on_player_stranger_appear,
                                ..notifications_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: notifications_view().notify_on_player_stranger_appear,
                }
                SettingsCheckbox {
                    label: "Friend appears",
                    on_value: move |notify_on_player_friend_appear| {
                        save_settings(SettingsData {
                            notifications: Notifications {
                                notify_on_player_friend_appear,
                                ..notifications_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: notifications_view().notify_on_player_friend_appear,
                }
                SettingsCheckbox {
                    label: "Detection fails or map changes",
                    on_value: move |notify_on_fail_or_change_map| {
                        save_settings(SettingsData {
                            notifications: Notifications {
                                notify_on_fail_or_change_map,
                                ..notifications_view.peek().clone()
                            },
                            ..settings_view.peek().clone()
                        });
                    },
                    value: notifications_view().notify_on_fail_or_change_map,
                }
            }
        }
    }
}

#[component]
fn SectionHotkeys(
    settings_view: Memo<SettingsData>,
    save_settings: EventHandler<SettingsData>,
) -> Element {
    #[component]
    fn Hotkey(
        label: &'static str,
        on_value: EventHandler<KeyBindingConfiguration>,
        value: KeyBindingConfiguration,
    ) -> Element {
        rsx! {
            div { class: "flex gap-2",
                KeyBindingInput {
                    label,
                    div_class: "flex-grow",
                    on_value: move |new_value: Option<KeyBinding>| {
                        on_value(KeyBindingConfiguration {
                            key: new_value.expect("not optional"),
                            ..value
                        });
                    },
                    value: Some(value.key),
                }
                SettingsCheckbox {
                    label: "Enabled",
                    on_value: move |enabled| {
                        on_value(KeyBindingConfiguration {
                            enabled,
                            ..value
                        });
                    },
                    value: value.enabled,
                }
            }
        }
    }

    rsx! {
        Section { name: "Hotkeys",
            div { class: "grid grid-cols-2 gap-3",
                Hotkey {
                    label: "Toggle start/stop actions",
                    on_value: move |toggle_actions_key| {
                        save_settings(SettingsData {
                            toggle_actions_key,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().toggle_actions_key,
                }
                Hotkey {
                    label: "Add platform",
                    on_value: move |platform_add_key| {
                        save_settings(SettingsData {
                            platform_add_key,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().platform_add_key,
                }
                Hotkey {
                    label: "Mark platform start",
                    on_value: move |platform_start_key| {
                        save_settings(SettingsData {
                            platform_start_key,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().platform_start_key,
                }
                Hotkey {
                    label: "Mark platform end",
                    on_value: move |platform_end_key| {
                        save_settings(SettingsData {
                            platform_end_key,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().platform_end_key,
                }
            }
        }
    }
}

#[component]
fn SectionRunStopCycle(
    settings_view: Memo<SettingsData>,
    save_settings: EventHandler<SettingsData>,
) -> Element {
    rsx! {
        Section { name: "Run/stop cycle",
            div { class: "grid grid-cols-3 gap-3",
                MillisInput {
                    label: "Run duration",
                    on_value: move |cycle_run_duration_millis| {
                        save_settings(SettingsData {
                            cycle_run_duration_millis,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().cycle_run_duration_millis,
                }
                MillisInput {
                    label: "Stop duration",
                    on_value: move |cycle_stop_duration_millis| {
                        save_settings(SettingsData {
                            cycle_stop_duration_millis,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().cycle_stop_duration_millis,
                }
                SettingsCheckbox {
                    label: "Enabled",
                    on_value: move |cycle_run_stop| {
                        save_settings(SettingsData {
                            cycle_run_stop,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().cycle_run_stop,
                }
            }
        }
    }
}

#[component]
fn SectionOthers(
    settings_view: Memo<SettingsData>,
    save_settings: EventHandler<SettingsData>,
) -> Element {
    let export_element_id = use_memo(|| Alphanumeric.sample_string(&mut rand::rng(), 8));
    let export = use_callback(move |_| {
        let js = format!(
            r#"
            const element = document.getElementById("{}");
            if (element === null) {{
                return;
            }}
            const json = await dioxus.recv();

            element.setAttribute("href", "data:application/json;charset=utf-8," + encodeURIComponent(json));
            element.setAttribute("download", "settings.json");
            element.click();
            "#,
            export_element_id(),
        );
        let eval = document::eval(js.as_str());
        let Ok(json) = serde_json::to_string_pretty(&*settings_view.peek()) else {
            return;
        };
        let _ = eval.send(json);
    });

    let import_element_id = use_memo(|| Alphanumeric.sample_string(&mut rand::rng(), 8));
    let import = use_callback(move |_| {
        let js = format!(
            r#"
            const element = document.getElementById("{}");
            if (element === null) {{
                return;
            }}
            element.click();
            "#,
            import_element_id()
        );
        document::eval(js.as_str());
    });
    let import_settings = use_callback(move |file| {
        let Some(id) = settings_view.peek().id else {
            return;
        };
        let Ok(file) = File::open(file) else {
            return;
        };
        let reader = BufReader::new(file);
        let Ok(mut settings) = serde_json::from_reader::<_, SettingsData>(reader) else {
            return;
        };
        settings.id = Some(id);
        save_settings(settings);
    });

    rsx! {
        Section { name: "Others",
            div { class: "grid grid-cols-2 gap-3",
                SettingsCheckbox {
                    label: "Enable rune solving",
                    on_value: move |enable_rune_solving| {
                        save_settings(SettingsData {
                            enable_rune_solving,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().enable_rune_solving,
                }
                div {}
                SettingsCheckbox {
                    label: "Stop actions on fail or map changed",
                    on_value: move |stop_on_fail_or_change_map| {
                        save_settings(SettingsData {
                            stop_on_fail_or_change_map,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().stop_on_fail_or_change_map,
                }
                SettingsCheckbox {
                    label: "Enable panic mode",
                    on_value: move |enable_panic_mode| {
                        save_settings(SettingsData {
                            enable_panic_mode,
                            ..settings_view.peek().clone()
                        });
                    },
                    value: settings_view().enable_panic_mode,
                }
                div {
                    a { id: export_element_id(), class: "w-0 h-0 invisible" }
                    Button {
                        class: "w-full",
                        text: "Export",
                        kind: ButtonKind::Primary,
                        on_click: move |_| {
                            export(());
                        },
                    }
                }
                div {
                    input {
                        id: import_element_id(),
                        class: "w-0 h-0 invisible",
                        r#type: "file",
                        accept: ".json",
                        name: "Settings JSON",
                        onchange: move |e| {
                            if let Some(file) = e
                                .data
                                .files()
                                .and_then(|engine| engine.files().into_iter().next())
                            {
                                import_settings(file);
                            }
                        },
                    }
                    Button {
                        class: "w-full",
                        text: "Import",
                        kind: ButtonKind::Primary,
                        on_click: move |_| {
                            import(());
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn SettingsSelect<T: 'static + Clone + PartialEq + Display>(
    label: &'static str,
    options: Vec<T>,
    on_select: EventHandler<(usize, T)>,
    selected: usize,
) -> Element {
    rsx! {
        Select {
            label,
            options,
            on_select,
            selected,
        }
    }
}

#[component]
fn SettingsEnumSelect<T: 'static + Clone + PartialEq + Display + IntoEnumIterator>(
    label: &'static str,
    #[props(default = false)] disabled: bool,
    on_select: EventHandler<T>,
    selected: T,
) -> Element {
    rsx! {
        EnumSelect {
            label,
            disabled,
            on_select,
            selected,
        }
    }
}

#[component]
fn SettingsCheckbox(
    label: &'static str,
    #[props(default = false)] disabled: bool,
    on_value: EventHandler<bool>,
    value: bool,
) -> Element {
    rsx! {
        Checkbox {
            label,
            input_class: "w-6",
            disabled,
            on_value,
            value,
        }
    }
}

#[component]
fn SettingsTextInput(
    text_label: String,
    button_label: String,
    on_value: EventHandler<String>,
    value: String,
) -> Element {
    let mut text = use_signal(String::default);

    use_effect(use_reactive!(|value| text.set(value)));

    rsx! {
        TextInput {
            label: text_label,
            on_value: move |new_text| {
                text.set(new_text);
            },
            value: text(),
        }
        div { class: "flex items-end",
            Button {
                text: button_label,
                kind: ButtonKind::Primary,
                on_click: move |_| {
                    on_value(text.peek().clone());
                },
                class: "w-full",
            }
        }
    }
}
