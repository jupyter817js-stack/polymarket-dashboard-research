//! Direct Gamma + data-api universe scan — fallback when the lifecycle-collector
//! hasn't yet ingested a candidate market.
//!
//! Strategy (mirrors rn1-live/src/gamma.rs::scan_universe):
//!   1. GET `/trades?limit=500` from data-api — distinct condition_ids whose
//!      slug starts with one of the universe prefixes (atp/wta).
//!   2. For each cid, GET `/markets?condition_ids=<cid>` from Gamma → enrich
//!      with volume, game_start, neg_risk, etc.
//!   3. Upsert into the `markets` table so the trader's primary path (fetch from
//!      DB) finds it on the next refresh.
//!
//! Why not just trust `/markets?tag=tennis`? Gamma's tag/slug-prefix filter is
//! unreliable for sports markets per the handoff (the recent-trades proxy is
//! how rn1-live did it in production).

use anyhow::{Context, Result};
use reqwest::Client;
use rn1_core::constants::{DATA_API, GAMMA_API};
use rn1_db as db;
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::HashSet;
use std::time::Duration;

const UA: &str = "rn1-trader/0.1 universe-scan";

#[derive(Debug, Deserialize)]
struct RawTrade {
    #[serde(rename = "conditionId")] condition_id: Option<String>,
    slug: Option<String>,
}

pub struct GammaScan {
    http: Client,
    api: rn1_data_api::DataApi,
}

impl GammaScan {
    pub fn new() -> Result<Self> {
        let http = Client::builder()
            .user_agent(UA).gzip(true).timeout(Duration::from_secs(20))
            .build().context("http builder")?;
        let api = rn1_data_api::DataApi::new().context("DataApi::new")?;
        Ok(Self { http, api })
    }

    /// Scan + upsert. Returns the number of markets newly written to the DB.
    ///
    /// Tries TWO paths and merges results:
    ///   A) data-api `/trades?limit=500` → distinct cids matching prefixes
    ///      (catches whatever RN1 / other actors are currently trading).
    ///   B) Gamma `/markets?active=true&closed=false&limit=500&order=volumeNum&ascending=false`
    ///      filtered by slug-head in `universe_prefixes`. Catches pre-game
    ///      markets that haven't seen trades yet — essential when RN1 is idle.
    pub async fn scan_and_upsert(
        &self,
        pool: &PgPool,
        universe_prefixes: &[String],
        limit: usize,
    ) -> Result<usize> {
        if universe_prefixes.is_empty() { return Ok(0); }

        let prefixes_lower: Vec<String> = universe_prefixes.iter()
            .map(|p| p.trim().to_lowercase()).collect();
        let slug_matches = |slug: &str| -> bool {
            let head = slug.split('-').next().unwrap_or("").to_lowercase();
            prefixes_lower.iter().any(|p| head == *p)
        };

        let mut cids: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        // Path A: data-api /trades (recent activity)
        match self.http.get(format!("{DATA_API}/trades?limit=500")).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(trades) = resp.json::<Vec<RawTrade>>().await {
                    for t in &trades {
                        let slug = t.slug.as_deref().unwrap_or("");
                        let cid = match &t.condition_id { Some(c) => c, None => continue };
                        if seen.contains(cid) || !slug_matches(slug) { continue; }
                        seen.insert(cid.clone());
                        cids.push(cid.clone());
                        if cids.len() >= limit { break; }
                    }
                }
            }
            Ok(r) => tracing::warn!(status=?r.status(), "data-api /trades non-200"),
            Err(e) => tracing::warn!(err=?e, "data-api /trades request failed"),
        }
        tracing::info!(path_a_cids = cids.len(),
            "universe scan A: recent-trades cids matching prefixes");

        // Path B: Gamma /markets (all active, slug-prefix matched) — covers
        // pre-game markets RN1 hasn't touched yet.
        match self.http.get(
            format!("{GAMMA_API}/markets?active=true&closed=false&limit=500&order=volumeNum&ascending=false")
        ).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(arr) = resp.json::<Vec<serde_json::Value>>().await {
                    for m in &arr {
                        let slug = m.get("slug").and_then(|v| v.as_str()).unwrap_or("");
                        let cid = match m.get("conditionId").and_then(|v| v.as_str()) {
                            Some(c) => c, None => continue,
                        };
                        if seen.contains(cid) || !slug_matches(slug) { continue; }
                        seen.insert(cid.to_string());
                        cids.push(cid.to_string());
                        if cids.len() >= limit { break; }
                    }
                }
            }
            Ok(r) => tracing::warn!(status=?r.status(), "gamma /markets non-200"),
            Err(e) => tracing::warn!(err=?e, "gamma /markets request failed"),
        }
        tracing::info!(total_cids = cids.len(),
            "universe scan: A+B combined cids matching prefixes");

        // Enrich + upsert
        let mut written = 0usize;
        for cid in &cids {
            match self.api.fetch_market(cid).await {
                Ok(Some(m)) => {
                    if let Err(e) = db::markets::upsert(pool, &m).await {
                        tracing::warn!(err=?e, %cid, "market upsert failed");
                    } else {
                        written += 1;
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(err=?e, %cid, "gamma fetch_market failed"),
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        Ok(written)
    }
}
