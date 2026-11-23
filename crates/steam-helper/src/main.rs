//! rimdeck-steam: runs one Steamworks Workshop action for rimdeck, then exits so Steam stops
//! showing RimWorld as running. Kept out of the frontends because it links the Steam API
//! library at load time.
//!
//! Usage: `rimdeck-steam <subscribe|unsubscribe|download> <published file id>...`
//! Output, one line each (parsed by `rimdeck_core::steam::parse_helper_line`):
//! `progress <text>`, `ok <id>`, `fail <id> <reason>`, `error <reason>`.

use std::sync::mpsc::{self, Receiver};
use std::thread::sleep;
use std::time::{Duration, Instant};

use steamworks::{Client, ItemState, PublishedFileId, SteamError};

const RIMWORLD_APP_ID: u32 = 294100;
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

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
                answer(&client, &rx).and_then(|()| installed(&client, item))
            }
            "unsubscribe" => {
                let (tx, rx) = mpsc::channel();
                ugc.unsubscribe_item(item, move |r| {
                    let _ = tx.send(r);
                });
                answer(&client, &rx)
            }
            _ if ugc.download_item(item, true) => installed(&client, item),
            _ => Err("Steam refused the download (is the item subscribed?)".into()),
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

/// Wait until Steam has the item installed and nothing is pending for it.
fn installed(client: &Client, item: PublishedFileId) -> Result<(), String> {
    let ugc = client.ugc();
    let start = Instant::now();
    let mut last = String::new();
    loop {
        client.run_callbacks();
        let state = ugc.item_state(item);
        let busy = state.intersects(
            ItemState::NEEDS_UPDATE | ItemState::DOWNLOADING | ItemState::DOWNLOAD_PENDING,
        );
        if state.contains(ItemState::INSTALLED) && !busy {
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
