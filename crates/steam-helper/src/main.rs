//! rimdeck-steam: runs one Steamworks Workshop action for rimdeck, then exits so Steam stops
//! showing RimWorld as running. Kept out of the frontends because it links the Steam API
//! library at load time.
//!
//! License: GPL-3.0-only, with this additional permission under GNU GPL version 3 section 7
//! (also in STEAMWORKS-EXCEPTION.md at https://github.com/CryoTheRenegade/rimdeck):
//!
//! If you modify this Program, or any covered work, by linking or combining it with the
//! Steamworks API library distributed by Valve Corporation (`steam_api64.dll`, `steam_api.dll`,
//! `libsteam_api.so` or `libsteam_api.dylib`, or a modified version of that library), containing
//! parts covered by the terms of the Steamworks SDK Access Agreement, the licensors of this
//! Program grant you additional permission to convey the resulting work.
//!
//! Usage: `rimdeck-steam <subscribe|unsubscribe|download> <published file id>...`
//! Output, one line each (parsed by `rimdeck_core::steam::parse_helper_line`):
//! `progress <text>`, `ok <id>`, `fail <id> <reason>`, `error <reason>`.

use std::sync::mpsc::{self, Receiver};
use std::thread::sleep;
use std::time::{Duration, Instant};

use steamworks::{Client, DownloadItemResult, ItemState, PublishedFileId, SteamError};

const RIMWORLD_APP_ID: u32 = 294100;
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Steam may post no DownloadItemResult for an item that is already current; after this
/// long an installed, idle item counts as done.
const SETTLE: Duration = Duration::from_secs(10);

fn main() {
    let mut args = std::env::args().skip(1);
    let action = args.next().unwrap_or_default();
    let ids: Vec<u64> = args.filter_map(|a| a.parse().ok()).collect();
    if !matches!(action.as_str(), "subscribe" | "unsubscribe" | "download") || ids.is_empty() {
        println!(
            "error usage: rimdeck-steam <subscribe|unsubscribe|download> <published file id>..."
        );
        std::process::exit(2);
    }
    let client = match Client::init_app(RIMWORLD_APP_ID) {
        Ok(client) => client,
        Err(e) => {
            println!("error the Steam app is not available ({e}); start Steam and sign in to an account that owns RimWorld");
            std::process::exit(1);
        }
    };
    let ugc = client.ugc();
    for (n, &id) in ids.iter().enumerate() {
        let item = PublishedFileId(id);
        println!("progress {} of {}: {action} {id}", n + 1, ids.len());
        let result = match action.as_str() {
            "subscribe" => {
                let (tx, rx) = mpsc::channel();
                ugc.subscribe_item(item, move |r| {
                    let _ = tx.send(r);
                });
                // Ask for the download explicitly: Steam can hold background Workshop
                // downloads while it counts RimWorld (this helper) as running.
                answer(&client, &rx).and_then(|()| download(&client, item))
            }
            "unsubscribe" => {
                let (tx, rx) = mpsc::channel();
                ugc.unsubscribe_item(item, move |r| {
                    let _ = tx.send(r);
                });
                answer(&client, &rx)
            }
            _ => download(&client, item),
        };
        match result {
            Ok(()) => println!("ok {id}"),
            Err(reason) => println!("fail {id} {reason}"),
        }
    }
}

/// Pump Steam callbacks until the call's result arrives.
fn answer(client: &Client, rx: &Receiver<Result<(), SteamError>>) -> Result<(), String> {
    let start = Instant::now();
    loop {
        client.run_callbacks();
        if let Ok(result) = rx.try_recv() {
            return result.map_err(|e| e.to_string());
        }
        if start.elapsed() > ANSWER_TIMEOUT {
            return Err("Steam did not answer".into());
        }
        sleep(Duration::from_millis(50));
    }
}

/// Ask Steam to download the item now and wait for its `DownloadItemResult`.
fn download(client: &Client, item: PublishedFileId) -> Result<(), String> {
    let ugc = client.ugc();
    let (tx, rx) = mpsc::channel();
    let _registration = client.register_callback(move |r: DownloadItemResult| {
        let _ = tx.send(r);
    });
    if !ugc.download_item(item, true) {
        return Err("Steam refused the download".into());
    }
    let start = Instant::now();
    let mut last = String::new();
    loop {
        client.run_callbacks();
        while let Ok(result) = rx.try_recv() {
            if result.published_file_id == item {
                return result.error.map_or(Ok(()), |e| Err(e.to_string()));
            }
        }
        let state = ugc.item_state(item);
        let busy = state.intersects(
            ItemState::NEEDS_UPDATE | ItemState::DOWNLOADING | ItemState::DOWNLOAD_PENDING,
        );
        if start.elapsed() > SETTLE && state.contains(ItemState::INSTALLED) && !busy {
            return Ok(());
        }
        if let Some(pct) = ugc
            .item_download_info(item)
            .and_then(|(done, total)| (done * 100).checked_div(total))
        {
            let line = format!("downloading {}: {pct}%", item.0);
            if line != last {
                println!("progress {line}");
                last = line;
            }
        }
        if start.elapsed() > DOWNLOAD_TIMEOUT {
            return Err("the download did not finish within 30 minutes".into());
        }
        sleep(Duration::from_millis(200));
    }
}
