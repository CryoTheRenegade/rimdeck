//! rimtui — RimWorld mod manager TUI (GPL-3.0-only).
//!
//! Inspired by RimSort (https://github.com/RimSort/RimSort).

mod app;
mod fs_browser;
mod preview;
mod ui;

use std::path::PathBuf;

use rimtui_core::settings::Settings;

const BUILD_DB_USAGE: &str = "usage: rimtui build-db [--out PATH] [--no-deps]

Merge every installed mod that has a Workshop ID into steamDB.json (packageId,
name, url, game versions). With a Steam WebAPI key (settings or
STEAM_WEB_API_KEY), Steam \"required items\" are fetched as dependencies.
Default output: the steamDB.json rimtui loads. Unknown fields are kept.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("build-db") => build_db(&args[1..]),
        Some("-h" | "--help") => {
            println!("usage: rimtui [build-db --help]");
            Ok(())
        }
        _ => app::run(),
    };
    if let Err(err) = result {
        eprintln!("{err:?}");
        std::process::exit(1);
    }
}

fn build_db(args: &[String]) -> anyhow::Result<()> {
    let mut out = None;
    let mut fetch_deps = true;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--out" => out = it.next().map(PathBuf::from),
            "--no-deps" => fetch_deps = false,
            "-h" | "--help" => {
                println!("{BUILD_DB_USAGE}");
                return Ok(());
            }
            other => anyhow::bail!("unknown argument {other}\n\n{BUILD_DB_USAGE}"),
        }
    }
    let settings = Settings::load()?;
    let out = out
        .or_else(Settings::steam_db_path)
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory; pass --out"))?;
    println!("Scanning mods and building {}...", out.display());
    let summary = rimtui_core::dbbuild::build_db_file(&settings, &out, fetch_deps)?;
    println!("{summary}");
    Ok(())
}
