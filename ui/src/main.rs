#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![feature(variant_count)]
#![feature(map_try_insert)]
#![feature(iter_intersperse)]

use std::{env::current_exe, io::stdout, string::ToString, sync::LazyLock};

use actions::Actions;
use backend::{Character, Minimap as MinimapData, Settings as SettingsData};
use characters::Characters;
#[cfg(debug_assertions)]
use debug::Debug;
use dioxus::{
    desktop::{
        WindowBuilder,
        tao::platform::windows::WindowBuilderExtWindows,
        wry::dpi::{PhysicalSize, Size},
    },
    prelude::*,
};
use fern::Dispatch;
use log::LevelFilter;
use minimap::Minimap;
use navigation::Navigation;
use rand::distr::{Alphanumeric, SampleString};
use settings::Settings;

mod actions;
mod button;
mod characters;
#[cfg(debug_assertions)]
mod debug;
mod icons;
mod inputs;
mod minimap;
mod navigation;
mod popup;
mod select;
mod settings;

const TAILWIND_CSS: Asset = asset!("public/tailwind.css");
const AUTO_NUMERIC_JS: Asset = asset!("assets/autoNumeric.min.js");
const TAB_ACTIONS: &str = "Actions";
const TAB_CHARACTERS: &str = "Characters";
const TAB_NAVIGATION: &str = "Navigation";
const TAB_SETTINGS: &str = "Settings";
#[cfg(debug_assertions)]
const TAB_DEBUG: &str = "Debug";

static TABS: LazyLock<Vec<String>> = LazyLock::new(|| {
    vec![
        TAB_ACTIONS.to_string(),
        TAB_CHARACTERS.to_string(),
        TAB_NAVIGATION.to_string(),
        TAB_SETTINGS.to_string(),
        #[cfg(debug_assertions)]
        TAB_DEBUG.to_string(),
    ]
});

fn main() {
    let level = if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339(std::time::SystemTime::now()),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(level)
        .chain(stdout())
        .chain(fern::log_file(current_exe().unwrap().parent().unwrap().join("log.txt")).unwrap())
        .apply()
        .unwrap();
    log_panics::init();

    backend::init();
    let window = WindowBuilder::new()
        .with_drag_and_drop(false)
        .with_inner_size(Size::new(PhysicalSize::new(1024, 483)))
        .with_min_inner_size(Size::new(PhysicalSize::new(320, 483)))
        .with_title(Alphanumeric.sample_string(&mut rand::rng(), 16));
    let cfg = dioxus::desktop::Config::default()
        .with_menu(None)
        .with_window(window);
    dioxus::LaunchBuilder::desktop().with_cfg(cfg).launch(App);
}

#[derive(Clone, Copy)]
pub struct AppState {
    minimap: Signal<Option<MinimapData>>,
    minimap_preset: Signal<Option<String>>,
    character: Signal<Option<Character>>,
    settings: Signal<Option<SettingsData>>,
    position: Signal<(i32, i32)>,
}

#[component]
fn App() -> Element {
    let mut selected_tab = use_signal(|| TAB_CHARACTERS.to_string());
    let mut script_loaded = use_signal(|| false);

    use_context_provider(|| AppState {
        minimap: Signal::new(None),
        minimap_preset: Signal::new(None),
        character: Signal::new(None),
        settings: Signal::new(None),
        position: Signal::new((0, 0)),
    });

    // Thanks dioxus
    use_future(move || async move {
        let mut eval = document::eval(
            r#"
            const scriptInterval = setInterval(async () => {
                try {
                    AutoNumeric;
                    await dioxus.send(true);
                    clearInterval(scriptInterval);
                } catch(_) { }
            }, 10);
        "#,
        );
        eval.recv::<bool>().await.unwrap();
        script_loaded.set(true);
    });

    rsx! {
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Script { src: AUTO_NUMERIC_JS }
        if script_loaded() {
            div { class: "flex min-w-3xl lg:min-w-5xl min-h-120 h-full",
                Minimap {}
                div { class: "flex-grow flex flex-col lg:flex-row z-1",
                    Tabs {
                        tabs: TABS.clone(),
                        on_select_tab: move |tab| {
                            selected_tab.set(tab);
                        },
                        selected_tab: selected_tab(),
                    }
                    div { class: "relative w-full h-full overflow-x-hidden overflow-y-auto pl-2 lg:pl-0",
                        match selected_tab().as_str() {
                            TAB_ACTIONS => rsx! {
                                Actions {}
                            },
                            TAB_CHARACTERS => rsx! {
                                Characters {}
                            },
                            TAB_SETTINGS => rsx! {
                                Settings {}
                            },
                            TAB_NAVIGATION => rsx! {
                                Navigation {}
                            },
                            #[cfg(debug_assertions)]
                            TAB_DEBUG => rsx! {
                                Debug {}
                            },
                            _ => unreachable!(),
                        }
                    }
                }
            }
        }
    }
}

#[derive(PartialEq, Props, Clone)]
struct TabsProps {
    tabs: Vec<String>,
    on_select_tab: EventHandler<String>,
    selected_tab: String,
}

#[component]
fn Tabs(
    TabsProps {
        tabs,
        on_select_tab,
        selected_tab,
    }: TabsProps,
) -> Element {
    rsx! {
        div { class: "flex flex-row lg:flex-col px-2 gap-3",
            for tab in tabs {
                Tab {
                    name: tab.clone(),
                    selected: selected_tab == tab,
                    on_click: move |_| {
                        on_select_tab(tab.clone());
                    },
                }
            }
        }
    }
}

#[component]
fn Tab(name: String, selected: bool, on_click: EventHandler) -> Element {
    let selected_class = if selected { "bg-gray-900" } else { "" };

    rsx! {
        button {
            class: "flex items-center pl-2 w-32 h-10 {selected_class} hover:bg-gray-900",
            onclick: move |_| {
                on_click(());
            },
            p { class: "title", {name} }
        }
    }
}
