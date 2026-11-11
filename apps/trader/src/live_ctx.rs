//! Live trading context — EOA signer + maker (proxy or EOA) + L2 API key.
//! Lifted from rn1-live/src/live.rs::LiveCtx + prepare_live.

use anyhow::{Context, Result};
use std::sync::Arc;

use alloy_primitives::Address;
use rn1_clob::{ClobClient, L2Auth};
use rn1_signer::{PolySigner, SIGNATURE_TYPE_EOA};

use crate::config::Config;

#[derive(Clone)]
pub struct LiveCtx {
    pub signer: Arc<PolySigner>,
    pub maker_address: Address,
    pub signature_type: u8,
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: String,
}

impl LiveCtx {
    pub fn l2_auth<'a>(&'a self, address_lower: &'a str) -> L2Auth<'a> {
        L2Auth {
            address: address_lower,
            api_key: &self.api_key,
            api_secret: &self.api_secret,
            passphrase: &self.passphrase,
        }
    }
    pub fn maker_hex(&self) -> String {
        format!("{:?}", self.maker_address).to_lowercase()
    }
    pub fn signer_field(&self) -> Address {
        match self.signature_type {
            SIGNATURE_TYPE_EOA => self.signer.address(),
            _ => self.maker_address,
        }
    }
}

pub async fn prepare_live(cfg: &Config, clob: &ClobClient) -> Result<LiveCtx> {
    if cfg.signer_pk.is_empty() {
        anyhow::bail!("POLYMARKET_SIGNER_PK is empty");
    }
    let signer = Arc::new(PolySigner::from_pk_hex(&cfg.signer_pk).context("load signer")?);

    let maker_address: Address = if let Some(p) = cfg.maker_address_override() {
        p.parse().context("parse POLYMARKET_PROXY_ADDRESS as Address")?
    } else {
        signer.address()
    };
    let signature_type = cfg.effective_signature_type();

    if maker_address != signer.address() {
        tracing::info!(
            eoa = %signer.address_lower(),
            proxy = %format!("{:?}", maker_address).to_lowercase(),
            signature_type,
            "live ctx: signer EOA signs orders for proxy maker"
        );
    } else {
        tracing::info!(
            eoa = %signer.address_lower(), signature_type,
            "live ctx: standalone EOA — funds must be held directly at signer"
        );
    }

    let (api_key, api_secret, passphrase) =
        if !cfg.api_key.is_empty() && !cfg.api_secret.is_empty() && !cfg.passphrase.is_empty() {
            (cfg.api_key.clone(), cfg.api_secret.clone(), cfg.passphrase.clone())
        } else {
            tracing::info!("L2 credentials not in env — deriving via /auth/derive-api-key");
            let existing = clob.get_api_key(&signer).await.context("get_api_key")?;
            let resp = if let Some(r) = existing {
                tracing::info!("found existing API key for {}", signer.address_lower());
                r
            } else {
                tracing::warn!("no existing API key — creating one via /auth/api-key");
                clob.derive_api_key(&signer).await.context("derive_api_key")?
            };
            tracing::warn!(
                "API key derived. Persist to .env:\nPOLY_API_KEY={}\nPOLY_API_SECRET={}\nPOLY_PASSPHRASE={}",
                resp.api_key, resp.secret, resp.passphrase
            );
            (resp.api_key, resp.secret, resp.passphrase)
        };

    Ok(LiveCtx {
        signer,
        maker_address,
        signature_type,
        api_key,
        api_secret,
        passphrase,
    })
}
