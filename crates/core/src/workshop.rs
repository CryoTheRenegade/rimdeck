use std::sync::mpsc::Sender;
use std::time::Duration;

use serde_json::Value;

use crate::constants::RIMWORLD_STEAM_APP_ID;
use crate::error::{Error, Result};
use crate::steamcmd::is_pfid;

const USER_AGENT: &str = concat!("rimdeck/", env!("CARGO_PKG_VERSION"));
const QUERY_FILES_URL: &str = "https://api.steampowered.com/IPublishedFileService/QueryFiles/v1/";
const GET_DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const GET_COLLECTION_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";
const GET_DETAILS_V2_URL: &str =
    "https://api.steampowered.com/IPublishedFileService/GetDetails/v1/";
const TIMEOUT_SECS: u64 = 60;
/// RankedByTextSearch
const QUERY_TYPE_TEXT: u32 = 11;
/// RankedByTotalUniqueSubscriptions (popular when the query is empty)
const QUERY_TYPE_POPULAR: u32 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopItem {
    pub published_file_id: String,
    pub title: String,
    pub subscriptions: u64,
    pub description: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub api_key: String,
    pub query: String,
}

#[derive(Debug, Clone)]
pub enum WorkshopMsg {
    Done(std::result::Result<Vec<WorkshopItem>, String>),
    /// Published file IDs of a collection's items, in collection order.
    Collection(std::result::Result<Vec<String>, String>),
}

pub fn spawn(req: SearchRequest, tx: Sender<WorkshopMsg>) {
    let tx_thread = tx.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("rimdeck-workshop".into())
        .spawn(move || {
            let result = search(&req).map_err(|e| redact(&e.to_string(), &req.api_key));
            let _ = tx_thread.send(WorkshopMsg::Done(result));
        })
    {
        let _ = tx.send(WorkshopMsg::Done(Err(format!(
            "could not start workshop search thread: {e}"
        ))));
    }
}

/// `123`, or a workshop URL with `?id=123`.
pub fn parse_collection_id(input: &str) -> Option<String> {
    let input = input.trim();
    if is_pfid(input) {
        return Some(input.to_string());
    }
    let (_, rest) = input.split_once("id=")?;
    let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
    is_pfid(&id).then_some(id)
}

/// Every collection URL or ID in `input` (separated by spaces, commas or new lines), deduped.
pub fn collection_ids(input: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for part in input.split(|c: char| c.is_whitespace() || c == ',') {
        if let Some(id) = parse_collection_id(part) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

pub fn spawn_collection(collection_id: String, tx: Sender<WorkshopMsg>) {
    let tx_thread = tx.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("rimdeck-collection".into())
        .spawn(move || {
            let result = collection_items(&collection_id).map_err(|e| e.to_string());
            let _ = tx_thread.send(WorkshopMsg::Collection(result));
        })
    {
        let _ = tx.send(WorkshopMsg::Collection(Err(format!(
            "could not start collection thread: {e}"
        ))));
    }
}

/// Every item of a collection in order, with nested collections (modpacks split into
/// parts) expanded in place. GetCollectionDetails needs no API key.
fn collection_items(collection_id: &str) -> Result<Vec<String>> {
    let client = blocking_client()?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    collect_items(&client, collection_id, &mut seen, &mut out, true)?;
    Ok(out)
}

fn collect_items(
    client: &reqwest::blocking::Client,
    collection_id: &str,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
    root: bool,
) -> Result<()> {
    if !seen.insert(collection_id.to_string()) {
        return Ok(()); // collections can include each other
    }
    let children = match collection_children(client, collection_id) {
        Ok(children) => children,
        // A private or deleted sub-collection shouldn't sink the whole pack.
        Err(e) if !root => {
            tracing::warn!("sub-collection {collection_id}: {e}");
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    for (is_collection, id) in children {
        if is_collection {
            collect_items(client, &id, seen, out, false)?;
        } else if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(())
}

fn collection_children(
    client: &reqwest::blocking::Client,
    collection_id: &str,
) -> Result<Vec<(bool, String)>> {
    let resp = client
        .post(GET_COLLECTION_URL)
        .form(&[("collectioncount", "1"), ("publishedfileids[0]", collection_id)])
        .send()
        .map_err(|e| Error::Workshop(e.to_string()))?;
    let status = resp.status();
    let body = resp.text().map_err(|e| Error::Workshop(e.to_string()))?;
    if !status.is_success() {
        return Err(Error::Workshop(format!(
            "GetCollectionDetails returned HTTP {status}"
        )));
    }
    parse_collection_children(&body)
}

/// Title and every item (nested collections expanded) of a Workshop collection.
pub fn fetch_pack(collection_id: &str) -> Result<(String, Vec<String>)> {
    let items = collection_items(collection_id)?;
    let title = get_details(&blocking_client()?, collection_id)
        .ok()
        .flatten()
        .map(|item| item.title)
        .unwrap_or_else(|| format!("Collection {collection_id}"));
    Ok((title, items))
}

/// Item IDs (not nested collections) sorted by `sortorder`.
pub fn parse_collection_json(text: &str) -> Result<Vec<String>> {
    Ok(parse_collection_children(text)?
        .into_iter()
        .filter_map(|(is_collection, id)| (!is_collection).then_some(id))
        .collect())
}

/// Children sorted by `sortorder` as `(is_collection, id)`; other file types are skipped.
fn parse_collection_children(text: &str) -> Result<Vec<(bool, String)>> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| Error::Workshop(format!("collection JSON: {e}")))?;
    let details = value
        .pointer("/response/collectiondetails/0")
        .ok_or_else(|| Error::Workshop("collection not found".into()))?;
    let Some(children) = details.get("children").and_then(Value::as_array) else {
        return Err(Error::Workshop(
            "not a collection, or it is private or empty".into(),
        ));
    };
    // filetype 0 = item, 2 = collection.
    let mut items: Vec<(u64, bool, String)> = children
        .iter()
        .filter_map(|c| {
            let is_collection = match c.get("filetype").and_then(json_u64).unwrap_or(0) {
                0 => false,
                2 => true,
                _ => return None,
            };
            let id = json_id(c.get("publishedfileid")?)?;
            let order = c.get("sortorder").and_then(json_u64).unwrap_or(u64::MAX);
            Some((order, is_collection, id))
        })
        .collect();
    items.sort_by_key(|(order, _, _)| *order);
    Ok(items
        .into_iter()
        .map(|(_, is_collection, id)| (is_collection, id))
        .collect())
}

/// Steam "required items" (children) per published file ID, 100 IDs per request.
pub fn required_items(
    api_key: &str,
    pfids: &[String],
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    let client = blocking_client()?;
    let mut out = std::collections::HashMap::new();
    for chunk in pfids.chunks(100) {
        let mut query: Vec<(String, String)> = vec![
            ("key".into(), api_key.to_string()),
            ("includechildren".into(), "true".into()),
        ];
        for (i, id) in chunk.iter().enumerate() {
            query.push((format!("publishedfileids[{i}]"), id.clone()));
        }
        let resp = client
            .get(GET_DETAILS_V2_URL)
            .query(&query)
            .send()
            .map_err(|e| Error::Workshop(redact(&e.to_string(), api_key)))?;
        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| Error::Workshop(redact(&e.to_string(), api_key)))?;
        if !status.is_success() {
            return Err(Error::Workshop(format!("GetDetails returned HTTP {status}")));
        }
        out.extend(parse_children_json(&body)?);
    }
    Ok(out)
}

/// `IPublishedFileService/GetDetails` with `includechildren` -> pfid -> child pfids.
pub fn parse_children_json(
    text: &str,
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| Error::Workshop(format!("GetDetails JSON: {e}")))?;
    let mut out = std::collections::HashMap::new();
    for item in publishedfiledetails(&value).into_iter().flatten() {
        let Some(id) = item.get("publishedfileid").and_then(json_id) else {
            continue;
        };
        let children = item
            .get("children")
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .filter_map(|c| c.get("publishedfileid").and_then(json_id))
                    .collect()
            })
            .unwrap_or_default();
        out.insert(id, children);
    }
    Ok(out)
}

pub fn search(req: &SearchRequest) -> Result<Vec<WorkshopItem>> {
    let client = blocking_client()?;
    let query = req.query.trim();
    if is_pfid(query) {
        match get_details(&client, query) {
            Ok(Some(item)) => return Ok(vec![item]),
            Ok(None) => {}
            Err(e) if req.api_key.trim().is_empty() => return Err(e),
            Err(_) => {}
        }
    }
    if req.api_key.trim().is_empty() {
        if is_pfid(query) {
            return Err(Error::Workshop(
                "Steam WebAPI did not return that published file ID".into(),
            ));
        }
        return Err(Error::Workshop(
            "set a Steam WebAPI key in path settings (p), or search by published file ID"
                .into(),
        ));
    }
    query_files(&client, &req.api_key, query)
}

fn blocking_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| Error::Workshop(e.to_string()))
}

fn query_files(
    client: &reqwest::blocking::Client,
    api_key: &str,
    query: &str,
) -> Result<Vec<WorkshopItem>> {
    let query_type = if query.is_empty() {
        QUERY_TYPE_POPULAR
    } else {
        QUERY_TYPE_TEXT
    };
    let form: Vec<(&str, String)> = vec![
        ("key", api_key.to_string()),
        ("query_type", query_type.to_string()),
        ("cursor", "*".into()),
        ("numperpage", "25".into()),
        ("creator_appid", RIMWORLD_STEAM_APP_ID.into()),
        ("appid", RIMWORLD_STEAM_APP_ID.into()),
        ("search_text", query.to_string()),
        ("filetype", "0".into()),
        ("match_all_tags", "false".into()),
        ("totalonly", "false".into()),
        ("ids_only", "false".into()),
        ("return_vote_data", "true".into()),
        ("return_tags", "true".into()),
        ("return_short_description", "true".into()),
        ("return_details", "true".into()),
        ("strip_description_bbcode", "true".into()),
        ("admin_query", "false".into()),
    ];
    let resp = client
        .post(QUERY_FILES_URL)
        .form(&form)
        .send()
        .map_err(|e| Error::Workshop(redact(&e.to_string(), api_key)))?;
    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| Error::Workshop(redact(&e.to_string(), api_key)))?;
    if !status.is_success() {
        return Err(Error::Workshop(format!(
            "QueryFiles returned HTTP {status}"
        )));
    }
    parse_query_files_json(&body)
}

/// Last update time (unix seconds) per published file ID, from Steam's public API (no key).
pub fn time_updated(pfids: &[String]) -> Result<std::collections::HashMap<String, u64>> {
    let client = blocking_client()?;
    let mut out = std::collections::HashMap::new();
    for chunk in pfids.chunks(100) {
        let mut form = vec![("itemcount".to_string(), chunk.len().to_string())];
        for (i, id) in chunk.iter().enumerate() {
            form.push((format!("publishedfileids[{i}]"), id.clone()));
        }
        let resp = client
            .post(GET_DETAILS_URL)
            .form(&form)
            .send()
            .map_err(|e| Error::Workshop(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().map_err(|e| Error::Workshop(e.to_string()))?;
        if !status.is_success() {
            return Err(Error::Workshop(format!(
                "GetPublishedFileDetails returned HTTP {status}"
            )));
        }
        out.extend(parse_time_updated_json(&body)?);
    }
    Ok(out)
}

/// `GetPublishedFileDetails` -> pfid -> `time_updated`; removed or hidden items are skipped.
pub fn parse_time_updated_json(text: &str) -> Result<std::collections::HashMap<String, u64>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Error::Workshop(format!("GetPublishedFileDetails JSON: {e}")))?;
    let mut out = std::collections::HashMap::new();
    for item in publishedfiledetails(&value).into_iter().flatten() {
        if item.get("result").and_then(Value::as_u64) != Some(1) {
            continue;
        }
        let id = item.get("publishedfileid").and_then(json_id);
        let time = item.get("time_updated").and_then(Value::as_u64);
        if let (Some(id), Some(time)) = (id, time) {
            out.insert(id, time);
        }
    }
    Ok(out)
}

fn get_details(
    client: &reqwest::blocking::Client,
    published_file_id: &str,
) -> Result<Option<WorkshopItem>> {
    let resp = client
        .post(GET_DETAILS_URL)
        .form(&[
            ("itemcount", "1"),
            ("publishedfileids[0]", published_file_id),
        ])
        .send()
        .map_err(|e| Error::Workshop(e.to_string()))?;
    let status = resp.status();
    let body = resp.text().map_err(|e| Error::Workshop(e.to_string()))?;
    if !status.is_success() {
        return Err(Error::Workshop(format!(
            "GetPublishedFileDetails returned HTTP {status}"
        )));
    }
    let items = parse_published_file_details_json(&body)?;
    Ok(items.into_iter().next())
}

pub fn parse_query_files_json(text: &str) -> Result<Vec<WorkshopItem>> {
    parse_details_array(text)
}

pub fn parse_published_file_details_json(text: &str) -> Result<Vec<WorkshopItem>> {
    parse_details_array(text)
}

fn parse_details_array(text: &str) -> Result<Vec<WorkshopItem>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Error::Workshop(format!("invalid JSON: {e}")))?;
    let Some(details) = publishedfiledetails(&value) else {
        if let Some(msg) = value
            .get("response")
            .and_then(|r| r.get("error"))
            .and_then(Value::as_str)
        {
            return Err(Error::Workshop(msg.to_string()));
        }
        return Ok(Vec::new());
    };
    let mut items = Vec::new();
    for entry in details {
        if let Some(item) = parse_item(entry) {
            items.push(item);
        }
    }
    Ok(items)
}

fn publishedfiledetails(value: &Value) -> Option<&Vec<Value>> {
    value
        .get("response")
        .and_then(|r| r.get("publishedfiledetails"))
        .and_then(Value::as_array)
        .or_else(|| value.get("publishedfiledetails").and_then(Value::as_array))
}

fn parse_item(value: &Value) -> Option<WorkshopItem> {
    let result = value.get("result").and_then(Value::as_u64).unwrap_or(1);
    if result != 1 && result != 0 {
        return None;
    }
    let published_file_id = json_id(value.get("publishedfileid")?)?;
    if !is_pfid(&published_file_id) {
        return None;
    }
    let title = json_string(value.get("title")).unwrap_or_else(|| format!("#{published_file_id}"));
    let description = json_string(value.get("short_description"))
        .or_else(|| json_string(value.get("file_description")))
        .or_else(|| json_string(value.get("description")))
        .unwrap_or_default();
    let subscriptions = value
        .get("subscriptions")
        .and_then(json_u64)
        .or_else(|| value.get("lifetime_subscriptions").and_then(json_u64))
        .unwrap_or(0);
    let tags = parse_tags(value.get("tags"));
    Some(WorkshopItem {
        published_file_id,
        title,
        subscriptions,
        description,
        tags,
    })
}

fn parse_tags(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(arr)) = value else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|t| {
            if let Some(s) = t.as_str() {
                return Some(s.to_string());
            }
            json_string(t.get("tag"))
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn json_id(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn json_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn json_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn redact(text: &str, secret: &str) -> String {
    let secret = secret.trim();
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "***")
}

pub fn format_subs(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}m", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{}k", n / 1_000)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUERY_FIXTURE: &str = r#"{
  "response": {
    "total": 2,
    "publishedfiledetails": [
      {
        "result": 1,
        "publishedfileid": "818773962",
        "title": "Harmony",
        "short_description": "A library for patching.",
        "subscriptions": 12345678,
        "tags": [{"tag": "1.5"}, {"tag": "Mod"}]
      },
      {
        "result": 1,
        "publishedfileid": 3396308787,
        "title": "Some Mod",
        "file_description": "Hello",
        "subscriptions": "42"
      }
    ]
  }
}"#;

    const DETAILS_FIXTURE: &str = r#"{
  "response": {
    "result": 1,
    "resultcount": 1,
    "publishedfiledetails": [
      {
        "publishedfileid": "818773962",
        "result": 1,
        "title": "Harmony",
        "description": "A library",
        "subscriptions": 99
      }
    ]
  }
}"#;

    #[test]
    fn parses_query_files_leniently() {
        let items = parse_query_files_json(QUERY_FIXTURE).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].published_file_id, "818773962");
        assert_eq!(items[0].title, "Harmony");
        assert_eq!(items[0].subscriptions, 12_345_678);
        assert!(items[0].tags.contains(&"1.5".to_string()));
        assert_eq!(items[1].published_file_id, "3396308787");
        assert_eq!(items[1].subscriptions, 42);
    }

    #[test]
    fn parses_get_details() {
        let items = parse_published_file_details_json(DETAILS_FIXTURE).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Harmony");
        assert_eq!(items[0].description, "A library");
    }

    #[test]
    fn empty_details_is_ok() {
        let items = parse_query_files_json(r#"{"response":{}}"#).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn skips_failed_result_codes() {
        let json = r#"{"response":{"publishedfiledetails":[
            {"result":9,"publishedfileid":"1","title":"gone"},
            {"result":1,"publishedfileid":"2","title":"ok"}
        ]}}"#;
        let items = parse_query_files_json(json).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].published_file_id, "2");
    }

    #[test]
    fn redacts_api_key_from_errors() {
        let key = "0123456789abcdef0123456789abcdef";
        let msg = format!("https://api.steampowered.com/?key={key}&x=1");
        let redacted = redact(&msg, key);
        assert!(!redacted.contains(key));
        assert!(redacted.contains("***"));
    }

    #[test]
    fn formats_subscription_counts() {
        assert_eq!(format_subs(12), "12");
        assert_eq!(format_subs(1500), "1.5k");
        assert_eq!(format_subs(12_345_678), "12.3m");
    }

    #[test]
    fn collection_ids_and_parse() {
        assert_eq!(parse_collection_id("2805123456").as_deref(), Some("2805123456"));
        assert_eq!(
            parse_collection_id(
                "https://steamcommunity.com/sharedfiles/filedetails/?id=2805123456&searchtext="
            )
            .as_deref(),
            Some("2805123456")
        );
        assert_eq!(parse_collection_id("hello"), None);
        let json = r#"{"response":{"result":1,"resultcount":1,"collectiondetails":[{
            "publishedfileid":"9","result":1,"children":[
              {"publishedfileid":"222","sortorder":2,"filetype":0},
              {"publishedfileid":"111","sortorder":1,"filetype":0},
              {"publishedfileid":"333","sortorder":3,"filetype":2}]}]}}"#;
        assert_eq!(parse_collection_json(json).unwrap(), vec!["111", "222"]);
        assert_eq!(
            collection_ids("https://steamcommunity.com/sharedfiles/filedetails/?id=111, 222
111"),
            vec!["111", "222"]
        );
        let nested = r#"{"response":{"collectiondetails":[{"result":1,"children":[
            {"publishedfileid":"900","sortorder":1,"filetype":2},
            {"publishedfileid":"111","sortorder":2,"filetype":0}]}]}}"#;
        assert_eq!(
            parse_collection_children(nested).unwrap(),
            vec![(true, "900".to_string()), (false, "111".to_string())]
        );
        let not_collection =
            r#"{"response":{"collectiondetails":[{"publishedfileid":"9","result":1}]}}"#;
        assert!(parse_collection_json(not_collection).is_err());
    }

    #[test]
    fn parses_required_items() {
        let json = r#"{"response":{"publishedfiledetails":[
            {"publishedfileid":"200","result":1,"children":[
                {"publishedfileid":"100","sortorder":1,"file_type":0}]},
            {"publishedfileid":"300","result":1}]}}"#;
        let map = parse_children_json(json).unwrap();
        assert_eq!(map["200"], vec!["100"]);
        assert!(map["300"].is_empty());
    }

    #[test]
    fn parses_time_updated_and_skips_missing_items() {
        let json = r#"{"response":{"publishedfiledetails":[
            {"publishedfileid":"100","result":1,"time_updated":1700000000},
            {"publishedfileid":"200","result":9}]}}"#;
        let map = parse_time_updated_json(json).unwrap();
        assert_eq!(map.get("100"), Some(&1700000000));
        assert!(!map.contains_key("200"));
    }
}
