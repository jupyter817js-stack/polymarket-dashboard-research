//! `/activity` endpoint — TRADE/REDEEM/MERGE/SPLIT history.

use anyhow::Result;
use rn1_core::constants::DATA_API;
use crate::{ActivityRow, DataApi};

impl DataApi {
    /// Activity 페이지 fetch. time-window + type 필터 지원.
    pub async fn activity(&self, wallet: &str, kind: Option<&str>, start: Option<i64>, end: Option<i64>,
                          limit: u32, offset: u32) -> Result<Vec<ActivityRow>> {
        let mut url = format!("{DATA_API}/activity?user={}&limit={}&offset={}", wallet, limit, offset);
        if let Some(k) = kind { url.push_str(&format!("&type={}", k)); }
        if let Some(s) = start { url.push_str(&format!("&start={}", s)); }
        if let Some(e) = end { url.push_str(&format!("&end={}", e)); }
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("activity http {}", resp.status());
        }
        let rows: Vec<ActivityRow> = resp.json().await?;
        Ok(rows)
    }

    /// 시간창 walk (offset 3000 cap 우회 — start/end 좁혀가며 호출).
    pub async fn activity_window(&self, wallet: &str, kind: &str,
                                  start: i64, end: i64) -> Result<Vec<ActivityRow>> {
        let mut out = Vec::new();
        let mut offset = 0u32;
        const LIMIT: u32 = 500;
        loop {
            let page = self.activity(wallet, Some(kind), Some(start), Some(end), LIMIT, offset).await?;
            let n = page.len() as u32;
            out.extend(page);
            if n < LIMIT { break; }
            offset += LIMIT;
            if offset >= 3000 { break; } // hard cap
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(out)
    }
}
