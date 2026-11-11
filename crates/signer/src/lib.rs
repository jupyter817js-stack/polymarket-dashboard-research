//! Polymarket auth + order signing (V2 / EIP-1271 wrapped) — port of
//! `rn1-live/src/signer.rs`. The POLY_1271 wrapped-hash logic is correct as
//! shipped to live; do not redo it.
//!
//! Three signature flows:
//!   1. L1 ClobAuth — EIP-712 over {address, timestamp, nonce, message}. Used
//!      to derive/rotate L2 API key.
//!   2. L2 HMAC — HMAC-SHA256 over `timestamp + method + path + body`, base64.
//!   3. Order signature (CTF Exchange V2):
//!      a. sigtypes 0/1/2 → standard EIP-712 over Order
//!      b. sigtype 3 (POLY_1271) → TypedDataSign wrapper + appended
//!         (appDomainSep | contentsHash | type_str | lenHex)

use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{eip712_domain, sol, SolStruct};
use anyhow::{Context, Result};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

pub const POLYGON_CHAIN_ID: u64 = 137;

pub const CTF_EXCHANGE_V1: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
pub const NEG_RISK_CTF_EXCHANGE_V1: &str = "0xC5d563A36AE78145C45a50134d48A1215220f80a";

pub const CTF_EXCHANGE_V2: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
pub const NEG_RISK_CTF_EXCHANGE_V2: &str = "0xe2222d279d744050d28e00520010520000310F59";

pub const CTF_EXCHANGE: &str = CTF_EXCHANGE_V2;
pub const NEG_RISK_CTF_EXCHANGE: &str = NEG_RISK_CTF_EXCHANGE_V2;

pub const SIDE_BUY: u8 = 0;
pub const SIDE_SELL: u8 = 1;

pub const SIGNATURE_TYPE_EOA: u8 = 0;
pub const SIGNATURE_TYPE_POLY_PROXY: u8 = 1;
pub const SIGNATURE_TYPE_POLY_GNOSIS_SAFE: u8 = 2;
pub const SIGNATURE_TYPE_POLY_1271: u8 = 3;

pub const CTF_DOMAIN_NAME: &str = "Polymarket CTF Exchange";
pub const CTF_DOMAIN_VERSION: &str = "2";

pub const ORDER_TYPE_STRING: &str = "Order(uint256 salt,address maker,address signer,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint8 side,uint8 signatureType,uint256 timestamp,bytes32 metadata,bytes32 builder)";

pub const DEPOSIT_WALLET_NAME: &str = "DepositWallet";
pub const DEPOSIT_WALLET_VERSION: &str = "1";

sol! {
    struct ClobAuth {
        address address;
        string timestamp;
        uint256 nonce;
        string message;
    }
}

sol! {
    #[derive(Debug)]
    struct Order {
        uint256 salt;
        address maker;
        address signer;
        uint256 tokenId;
        uint256 makerAmount;
        uint256 takerAmount;
        uint8 side;
        uint8 signatureType;
        uint256 timestamp;
        bytes32 metadata;
        bytes32 builder;
    }

    struct TypedDataSign {
        Order contents;
        string name;
        string version;
        uint256 chainId;
        address verifyingContract;
        bytes32 salt;
    }
}

#[derive(Clone)]
pub struct PolySigner {
    signer: PrivateKeySigner,
    address: Address,
}

impl PolySigner {
    pub fn from_pk_hex(pk: &str) -> Result<Self> {
        let pk = pk.strip_prefix("0x").unwrap_or(pk);
        let signer: PrivateKeySigner = pk.parse().context("parse private key")?;
        let address = signer.address();
        Ok(Self { signer, address })
    }

    pub fn address(&self) -> Address { self.address }
    pub fn address_lower(&self) -> String { format!("{:?}", self.address).to_lowercase() }

    pub fn sign_clob_auth(&self, timestamp: i64, nonce: u64) -> Result<String> {
        let domain = eip712_domain! {
            name: "ClobAuthDomain",
            version: "1",
            chain_id: POLYGON_CHAIN_ID,
        };
        let msg = ClobAuth {
            address: self.address,
            timestamp: timestamp.to_string(),
            nonce: U256::from(nonce),
            message: "This message attests that I control the given wallet".to_string(),
        };
        let hash: B256 = msg.eip712_signing_hash(&domain);
        let sig = self.signer.sign_hash_sync(&hash).context("sign clob auth")?;
        Ok(format!("0x{}", hex::encode(sig.as_bytes())))
    }

    pub fn sign_order(&self, order: &Order, verifying_contract: Address) -> Result<String> {
        match order.signatureType {
            SIGNATURE_TYPE_POLY_1271 => self.sign_order_poly1271(order, verifying_contract),
            _ => self.sign_order_standard(order, verifying_contract),
        }
    }

    fn sign_order_standard(&self, order: &Order, verifying_contract: Address) -> Result<String> {
        let domain = eip712_domain! {
            name: CTF_DOMAIN_NAME,
            version: CTF_DOMAIN_VERSION,
            chain_id: POLYGON_CHAIN_ID,
            verifying_contract: verifying_contract,
        };
        let hash: B256 = order.eip712_signing_hash(&domain);
        let sig = self.signer.sign_hash_sync(&hash).context("sign order v2 standard")?;
        Ok(format!("0x{}", hex::encode(sig.as_bytes())))
    }

    fn sign_order_poly1271(&self, order: &Order, verifying_contract: Address) -> Result<String> {
        let app_domain_sep = compute_app_domain_separator(verifying_contract);
        let contents_hash = compute_order_struct_hash(order);
        let tds_struct_hash = compute_typed_data_sign_struct_hash(&contents_hash, order.signer);

        let mut buf = Vec::with_capacity(2 + 32 + 32);
        buf.push(0x19); buf.push(0x01);
        buf.extend_from_slice(app_domain_sep.as_slice());
        buf.extend_from_slice(tds_struct_hash.as_slice());
        let inner_hash = keccak256(&buf);

        let inner_sig = self.signer.sign_hash_sync(&inner_hash).context("sign poly1271 inner")?;
        let inner_sig_hex = hex::encode(inner_sig.as_bytes());

        let type_str_bytes = ORDER_TYPE_STRING.as_bytes();
        let type_str_hex = hex::encode(type_str_bytes);
        let len_hex = format!("{:04x}", type_str_bytes.len());

        Ok(format!(
            "0x{}{}{}{}{}",
            inner_sig_hex,
            hex::encode(app_domain_sep.as_slice()),
            hex::encode(contents_hash.as_slice()),
            type_str_hex,
            len_hex
        ))
    }
}

fn compute_typed_data_sign_struct_hash(
    contents_hash: &B256,
    typed_data_sign_verifying_contract: Address,
) -> B256 {
    const TYPED_DATA_SIGN_HEAD: &str = "TypedDataSign(Order contents,string name,string version,uint256 chainId,address verifyingContract,bytes32 salt)";
    let mut type_string = String::with_capacity(TYPED_DATA_SIGN_HEAD.len() + ORDER_TYPE_STRING.len());
    type_string.push_str(TYPED_DATA_SIGN_HEAD);
    type_string.push_str(ORDER_TYPE_STRING);
    let type_hash = keccak256(type_string.as_bytes());

    let dw_name_hash    = keccak256(DEPOSIT_WALLET_NAME.as_bytes());
    let dw_version_hash = keccak256(DEPOSIT_WALLET_VERSION.as_bytes());

    let mut buf = Vec::with_capacity(32 * 7);
    buf.extend_from_slice(type_hash.as_slice());
    buf.extend_from_slice(contents_hash.as_slice());
    buf.extend_from_slice(dw_name_hash.as_slice());
    buf.extend_from_slice(dw_version_hash.as_slice());
    buf.extend_from_slice(&U256::from(POLYGON_CHAIN_ID).to_be_bytes::<32>());
    buf.extend_from_slice(&[0u8; 12]);
    buf.extend_from_slice(typed_data_sign_verifying_contract.as_slice());
    buf.extend_from_slice(&[0u8; 32]);

    keccak256(&buf)
}

fn compute_app_domain_separator(verifying_contract: Address) -> B256 {
    let domain_type_hash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
    );
    let name_hash    = keccak256(CTF_DOMAIN_NAME.as_bytes());
    let version_hash = keccak256(CTF_DOMAIN_VERSION.as_bytes());

    let mut buf = Vec::with_capacity(160);
    buf.extend_from_slice(domain_type_hash.as_slice());
    buf.extend_from_slice(name_hash.as_slice());
    buf.extend_from_slice(version_hash.as_slice());
    buf.extend_from_slice(&U256::from(POLYGON_CHAIN_ID).to_be_bytes::<32>());
    buf.extend_from_slice(&[0u8; 12]);
    buf.extend_from_slice(verifying_contract.as_slice());
    keccak256(&buf)
}

fn compute_order_struct_hash(order: &Order) -> B256 {
    let order_type_hash = keccak256(ORDER_TYPE_STRING.as_bytes());

    let mut buf = Vec::with_capacity(32 * 12);
    buf.extend_from_slice(order_type_hash.as_slice());
    buf.extend_from_slice(&order.salt.to_be_bytes::<32>());
    buf.extend_from_slice(&[0u8; 12]); buf.extend_from_slice(order.maker.as_slice());
    buf.extend_from_slice(&[0u8; 12]); buf.extend_from_slice(order.signer.as_slice());
    buf.extend_from_slice(&order.tokenId.to_be_bytes::<32>());
    buf.extend_from_slice(&order.makerAmount.to_be_bytes::<32>());
    buf.extend_from_slice(&order.takerAmount.to_be_bytes::<32>());
    let mut tmp = [0u8; 32]; tmp[31] = order.side;           buf.extend_from_slice(&tmp);
    let mut tmp = [0u8; 32]; tmp[31] = order.signatureType;  buf.extend_from_slice(&tmp);
    buf.extend_from_slice(&order.timestamp.to_be_bytes::<32>());
    buf.extend_from_slice(order.metadata.as_slice());
    buf.extend_from_slice(order.builder.as_slice());

    keccak256(&buf)
}

pub fn hmac_l2_signature(
    secret_b64: &str, timestamp: i64, method: &str, path: &str, body: &str,
) -> Result<String> {
    let secret = base64::engine::general_purpose::URL_SAFE
        .decode(secret_b64).context("base64-decode api-secret (URL_SAFE)")?;
    let mut mac = <Hmac<Sha256>>::new_from_slice(&secret).context("hmac init")?;
    let ts = timestamp.to_string();
    let msg = format!("{ts}{method}{path}{body}");
    mac.update(msg.as_bytes());
    Ok(base64::engine::general_purpose::URL_SAFE.encode(mac.finalize().into_bytes()))
}

pub fn now_unix_secs() -> i64 { chrono::Utc::now().timestamp() }
pub fn now_unix_millis() -> i64 { chrono::Utc::now().timestamp_millis() }

/// Polymarket clob-client-v2 salt: a JS-Number-safe integer in [0, now_ms].
pub fn random_salt() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    let random_u64 = u64::from_be_bytes(bytes);
    let now_ms = chrono::Utc::now().timestamp_millis() as u64;
    let r_frac = (random_u64 as f64) / (u64::MAX as f64);
    let salt = (r_frac * (now_ms as f64)).round() as u64;
    salt.to_string()
}

pub fn exchange_address_for(neg_risk: bool) -> Address {
    let s = if neg_risk { NEG_RISK_CTF_EXCHANGE_V2 } else { CTF_EXCHANGE_V2 };
    s.parse().expect("hardcoded exchange address")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_domain_sep_v2_ctf_matches_reference() {
        let addr: Address = "0xE111180000d2663C0091e4f400237545B87B996B".parse().unwrap();
        let got = compute_app_domain_separator(addr);
        assert_eq!(
            hex::encode(got.as_slice()),
            "3264e159346253e26a64e00b69032db0e7d32f94628de3e6eecb50304d7af3d2",
        );
    }

    #[test]
    fn hmac_matches_pyclobclient_reference() {
        let got = hmac_l2_signature(
            "hm4EDy1M769ujXjFdp8k-M1H0aJZa2UNHQGYSt-eY20=",
            1700000000, "GET", "/test", "",
        ).unwrap();
        assert_eq!(got, "rCBAGWcGRk3AZ6oDdahhOkDcV_FpaOTisKMCMTonDTs=");
    }

    #[test]
    fn order_type_string_length_is_186() {
        assert_eq!(ORDER_TYPE_STRING.len(), 186);
    }
}
