//! Thin client for the deployed Worker's AI endpoints. The daemon calls these
//! and caches the text in the local SQLite store ([`crate::store`]); nothing
//! here talks to DeepSeek — all the prompts/caching live server-side.
//!
//! Base URL override: `CTA_AI_BASE` (default = the production worker).

use anyhow::{anyhow, Result};
use serde::Deserialize;

fn base() -> String {
    std::env::var("CTA_AI_BASE")
        .unwrap_or_else(|_| "https://cta-track-grid.felipe-debene.workers.dev".into())
}

/// Shape of every AI endpoint response (`{summary, count, ...}` or `{error}`).
#[derive(Deserialize, Default)]
pub struct AiResp {
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub count: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

async fn get(client: &reqwest::Client, path: &str) -> Result<AiResp> {
    let url = format!("{}{}", base(), path);
    let resp: AiResp = client.get(url).send().await?.json().await?;
    if let Some(e) = &resp.error {
        return Err(anyhow!("{e}"));
    }
    if resp.summary.as_deref().unwrap_or("").trim().is_empty() {
        return Err(anyhow!("empty summary"));
    }
    Ok(resp)
}

pub async fn fetch_dispatch(client: &reqwest::Client) -> Result<AiResp> {
    get(client, "/api/feed/narration").await
}

pub async fn fetch_sitrep(client: &reqwest::Client, mapid: &str, name: &str) -> Result<AiResp> {
    let path = format!("/api/alerts/summary?station={}&stn={}", urlenc(mapid), urlenc(name));
    get(client, &path).await
}

pub async fn fetch_events(client: &reqwest::Client) -> Result<AiResp> {
    get(client, "/api/events/advisory").await
}

/// Minimal percent-encoding for query values (station names contain spaces, `/`).
fn urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
