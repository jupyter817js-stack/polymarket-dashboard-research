//! `/positions` endpoint.

use anyhow::Result;
use rn1_core::constants::DATA_API;
use crate::{DataApi, PositionRow};

impl DataApi {
    /// 모든 보유 positions (size > 0 + sizeThreshold=0 으로 청산 직전도 포함).
    pub async fn positions_all(&self, wallet: &str) -> Result<Vec<PositionRow>> {
        let mut out = Vec::new();
        let mut offset = 0u32;
        const LIMIT: u32 = 500;
        loop {
            let url = format!("{DATA_API}/positions?user={}&limit={}&offset={}&sizeThreshold=0",
                wallet, LIMIT, offset);
            let resp = self.client.get(&url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("positions http {}", resp.status());
            }
            let page: Vec<PositionRow> = resp.json().await?;
            let n = page.len();
            out.extend(page);
            if n < LIMIT as usize { break; }
            offset += LIMIT;
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        Ok(out)
    }
}
