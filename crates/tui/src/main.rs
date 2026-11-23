//! rimtui — RimWorld mod manager TUI (GPL-3.0-only).
//!
//! Inspired by RimSort (https://github.com/RimSort/RimSort).

mod app;
mod fs_browser;
mod preview;
mod ui;

fn main() {
    if let Err(err) = app::run() {
        eprintln!("{err:?}");
        std::process::exit(1);
    }
}
