#![forbid(unsafe_code)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod app;
mod components;
mod server_fns;

fn main() {
    dioxus::launch(app::App);
}
