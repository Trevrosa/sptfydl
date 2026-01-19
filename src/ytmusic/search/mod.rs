pub mod parsing;

use std::{sync::OnceLock, time::Instant};

use anyhow::anyhow;
use chrono::{Datelike, Utc};
use regex::Regex;
use reqwest::{blocking::Response, header::HeaderMap};
use serde_json::{Value, json};
use tracing::{debug, trace};

use crate::CLIENT;

const SEARCH_API: &str = "https://music.youtube.com/youtubei/v1/search";
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:88.0) Gecko/20100101 Firefox/88.0";

/// The base context object that youtube music requires in order for api calls to work.
///
/// <https://github.com/sigma67/ytmusicapi//blob/a979691bb03c1cb5e7e39985bbd4014187940d68/ytmusicapi/helpers.py#L30>
#[inline]
fn base_context() -> Value {
    let now = Utc::now();
    // time.strftime("%Y%m%d", time.gmtime())
    let now = format!("{}{:02}{:02}", now.year(), now.month(), now.day());
    let client_version = format!("1.{now}.01.00");

    json!({
        "context": {
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": client_version,
                "hl": "en"
            },
            "user": {}
        }
    })
}

/// The base headers that youtube music requires in order for api calls to work.
///
/// <https://github.com/sigma67/ytmusicapi//blob/a979691bb03c1cb5e7e39985bbd4014187940d68/ytmusicapi/helpers.py#L17>
#[inline]
fn base_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();

    headers.insert("Accept", "*/*".parse().unwrap());
    headers.insert("Accept-Encoding", "gzip, deflate".parse().unwrap());
    // headers.insert("Content-Encoding", "gzip".parse().unwrap());
    headers.insert("Origin", "https://music.youtube.com".parse().unwrap());
    headers.insert("X-Origin", "https://music.youtube.com".parse().unwrap());
    headers.insert("Referer", "https://music.youtube.com".parse().unwrap());
    headers.insert("User-Agent", USER_AGENT.parse().unwrap());

    headers
}

static COOKIES: OnceLock<String> = OnceLock::new();

/// Send a get request to the base youtube music url.
fn get_base() -> anyhow::Result<Response> {
    let resp = CLIENT
        .get("https://music.youtube.com")
        .headers(base_headers())
        .header("Cookie", "SOCS=CAI")
        .send()?;

    trace!("sending normal req to ytm");

    if !resp.status().is_success() {
        return Err(anyhow!(
            "music.youtube.com gave {}: {:#?}",
            resp.status(),
            resp.text()
        ));
    }

    Ok(resp)
}

/// Extract the `X-Goog-Visitor-Id` from a normal request to youtube music.
///
/// <https://github.com/sigma67/ytmusicapi//blob/a979691bb03c1cb5e7e39985bbd4014187940d68/ytmusicapi/helpers.py#L42>
fn parse_visitor_id(resp: &str) -> anyhow::Result<String> {
    let start = Instant::now();

    // original: r"ytcfg\.set\s*\(\s*({.+?})\s*\)\s*;"
    // use (?s) to match across lines
    let re = Regex::new(r"ytcfg\.set\s*\(\s*(\{.+?\})\s*\)\s*;").unwrap();

    trace!("finding ytcfg blob");
    let cfg_blob = re
        .captures(resp)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str())
        .ok_or(anyhow!("failed to find cfg blob"))?;

    trace!("parsing it as json");
    let cfg: Value = serde_json::from_str(cfg_blob)?;

    let Some(visitor_id) = cfg.get("VISITOR_DATA") else {
        return Err(anyhow!("failed to find VISITOR_DATA from cfg"));
    };

    debug!("found visitor id! (took {:?})", start.elapsed());
    Ok(visitor_id
        .as_str()
        .ok_or(anyhow!("VISITOR_DATA not str"))?
        .to_string())
}

static VISITOR_ID: OnceLock<String> = OnceLock::new();

/// Search youtube music by `query`, using authentication `auth`.
pub fn search(query: impl AsRef<str>, auth: &str) -> anyhow::Result<Response> {
    let query = query.as_ref();

    // the json payload
    let mut body: Value = base_context();

    if let Value::Object(ref mut map) = body {
        map.insert("query".to_string(), Value::String(query.to_string()));
    }

    let base_resp = get_base()?;

    // we want to copy the cookies youtube music sends us and keep them.
    let cookies = COOKIES.get_or_init(|| {
        let mut cookies = "SOCS=CAI".to_string();

        for (_n, v) in base_resp
            .headers()
            .iter()
            .filter(|(n, _v)| n.as_str() == "set-cookie")
        {
            let cookie_str = v.to_str().unwrap().split(';').next().unwrap();
            cookies += "; ";
            cookies += cookie_str;
        }

        debug!("saved cookies");
        cookies
    });

    let base_resp = base_resp.text().unwrap();

    let visitor_id = VISITOR_ID.get_or_init(|| parse_visitor_id(&base_resp).unwrap());

    let resp = CLIENT
        .post(SEARCH_API)
        // https://github.com/sigma67/ytmusicapi//blob/14a575e1685c21474e03461cbcccc1bdff44b47e/ytmusicapi/ytmusic.py#L169
        .header("Authentication", auth)
        // https://github.com/sigma67/ytmusicapi//blob/fe95f5974efd7ba8b87ba030a1f528afe41a5a31/ytmusicapi/constants.py#L3
        .query(&[("alt", "json")])
        .json(&body)
        .headers(base_headers())
        .header("Cookie", cookies)
        // https://github.com/sigma67/ytmusicapi//blob/14a575e1685c21474e03461cbcccc1bdff44b47e/ytmusicapi/ytmusic.py#L164
        .header("X-Goog-Visitor-Id", visitor_id)
        // // https://github.com/sigma67/ytmusicapi//blob/14a575e1685c21474e03461cbcccc1bdff44b47e/ytmusicapi/ytmusic.py#L180
        // .header("X-Goog-Request-Time", Utc::now().timestamp().to_string())
        .send()?;

    Ok(resp)
}
