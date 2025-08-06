use dioxus::prelude::*;

#[component]
pub fn DetailsIcon(class: String) -> Element {
    rsx! {
        svg {
            class,
            width: "24px",
            height: "24px",
            view_box: "0 0 24 24",
            path { d: "M20 3H4c-1.103 0-2 .897-2 2v14c0 1.103.897 2 2 2h16c1.103 0 2-.897 2-2V5c0-1.103-.897-2-2-2zM4 19V5h16l.002 14H4z" }
            path { d: "M6 7h12v2H6zm0 4h12v2H6zm0 4h6v2H6z" }
        }
    }
}

#[component]
pub fn EyePasswordShowIcon(class: String) -> Element {
    rsx! {
        svg {
            class,
            fill: "none",
            width: "24px",
            height: "24px",
            view_box: "0 0 24 24",
            path {
                stroke: "#000",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                stroke_width: "2",
                d: "M1 12s4-8 11-8 11 8 11 8M1 12s4 8 11 8 11-8 11-8",
            }
            circle {
                cx: "12",
                cy: "12",
                r: "3",
                stroke: "#000",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                stroke_width: "2",
            }
        }
    }
}

#[component]
pub fn EyePasswordHideIcon(class: String) -> Element {
    rsx! {
        svg {
            class,
            fill: "none",
            width: "24px",
            height: "24px",
            view_box: "0 0 24 24",
            path {
                stroke: "currentColor",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                stroke_width: "2",
                d: "m2 2 20 20M6.713 6.723C3.665 8.795 2 12 2 12s3.636 7 10 7c2.05 0 3.817-.727 5.271-1.712M11 5.058A8.595 8.595 0 0 1 12 5c6.364 0 10 7 10 7s-.692 1.332-2 2.834",
            }
            path {
                stroke: "#000",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                stroke_width: "2",
                d: "M14 14.236a3 3 0 0 1-4.13-4.348",
            }
        }
    }
}

#[component]
pub fn XIcon(class: String) -> Element {
    rsx! {
        svg {
            class,
            width: "24px",
            height: "24px",
            view_box: "0 0 24 24",
            path { d: "m24 20.188-8.315-8.209 8.2-8.282L20.188 0l-8.212 8.318L3.666.115 0 3.781l8.321 8.24-8.206 8.313L3.781 24l8.237-8.318 8.285 8.203z" }
        }
    }
}

#[component()]
pub fn PositionIcon(class: String) -> Element {
    rsx! {
        svg {
            class,
            width: "24px",
            height: "24px",
            view_box: "0 0 434.174 434.174",
            path { d: "M217.087 119.397c-24.813 0-45 20.187-45 45s20.187 45 45 45 45-20.187 45-45-20.186-45-45-45z" }
            path { d: "M217.087 0c-91.874 0-166.62 74.745-166.62 166.619 0 38.93 13.421 74.781 35.878 103.177l130.742 164.378 130.742-164.378c22.457-28.396 35.878-64.247 35.878-103.177C383.707 74.745 308.961 0 217.087 0zm0 239.397c-41.355 0-75-33.645-75-75s33.645-75 75-75 75 33.645 75 75-33.644 75-75 75z" }
        }
    }
}

#[component]
pub fn UpArrowIcon(class: String) -> Element {
    rsx! {
        svg {
            class,
            width: "24px",
            height: "24px",
            view_box: "0 0 24 24",
            path { d: "M3 19h18a1.002 1.002 0 0 0 .823-1.569l-9-13c-.373-.539-1.271-.539-1.645 0l-9 13A.999.999 0 0 0 3 19z" }
        }
    }
}

#[component]
pub fn DownArrowIcon(class: String) -> Element {
    rsx! {
        svg {
            class,
            width: "24px",
            height: "24px",
            view_box: "0 0 24 24",
            path { d: "M11.178 19.569a.998.998 0 0 0 1.644 0l9-13A.999.999 0 0 0 21 5H3a1.002 1.002 0 0 0-.822 1.569l9 13z" }
        }
    }
}
