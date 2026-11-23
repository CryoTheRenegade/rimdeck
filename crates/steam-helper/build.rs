//! Put the Steam API library next to the helper exe; the helper links it at load time and
//! would not start without it. steamworks-sys leaves it in its own OUT_DIR only.

use std::path::PathBuf;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = env::var("TARGET").unwrap_or_default();
    let lib = if target.contains("windows") {
        "steam_api64.dll"
    } else if target.contains("darwin") {
        "libsteam_api.dylib"
    } else {
        "libsteam_api.so"
    };
    // OUT_DIR is <profile>/build/rimdeck-steam-<hash>/out, or <profile>/build/rimdeck-steam/<hash>/out
    // with newer cargo; the exe goes in <profile>.
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let Some(build_dir) = out
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "build"))
    else {
        return;
    };
    let Some(profile_dir) = build_dir.parent() else {
        return;
    };
    let mut candidates = Vec::new();
    for entry in fs::read_dir(build_dir).into_iter().flatten().flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("steamworks-sys")
        {
            continue;
        }
        candidates.push(entry.path().join("out").join(lib));
        for hashed in fs::read_dir(entry.path()).into_iter().flatten().flatten() {
            candidates.push(hashed.path().join("out").join(lib));
        }
    }
    let newest = candidates
        .into_iter()
        .filter(|p| p.is_file())
        .max_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    match newest {
        Some(src) => {
            if let Err(e) = fs::copy(&src, profile_dir.join(lib)) {
                println!("cargo:warning=could not copy {lib} next to rimdeck-steam: {e}");
            }
        }
        None => println!("cargo:warning={lib} not found; rimdeck-steam will not start without it"),
    }
}
