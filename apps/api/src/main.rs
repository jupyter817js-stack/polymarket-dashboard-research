//! rn1-api — Frontend 가 호출하는 REST + SSE 서버.
//!
//! 라우트:
//!   GET  /api/health
//!   GET  /api/portfolio/positions   — RN1 positions (event tree 그룹화 옵션)
//!   GET  /api/portfolio/orders      — open orders (M4+ 까지 항상 빈 배열)
//!   GET  /api/portfolio/history     — 최근 fills (TRADE)
//!   GET  /api/portfolio/analysis    — 전략 분해 (forensic + 라이브 누적)
//!   GET  /api/portfolio/redemptions — REDEEM / MERGE 이력
//!   GET  /api/live/summary          — 우리 봇 라이브 KPI (M4+ 까지 placeholder)
//!   GET  /api/system/status         — heartbeats + sync cursors
//!   GET  /api/stream                — SSE: 모든 position/fill/heartbeat 변화

use anyhow::Result;
use axum::{
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use clap::Parser;
use rn1_db as db;
use serde::Serialize;
use sqlx::PgPool;
use std::{net::SocketAddr, sync::Arc};
use tower_http::{cors::CorsLayer, compression::CompressionLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

mod handlers;

#[derive(Parser, Debug, Clone)]
#[command(name = "rn1-api")]
struct Args {
    #[arg(long, env = "RN1_API_PORT", default_value_t = 8000)]
    port: u16,
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,rn1=debug,tower_http=info")))
        .compact()
        .init();
    let args = Args::parse();
    if let Some(u) = &args.database_url { std::env::set_var("DATABASE_URL", u); }
    let pool = db::pool_from_env().await?;
    db::migrate(&pool).await?;

    let state = Arc::new(AppState { pool });

    let app = Router::new()
        .route("/api/health", get(handlers::health))
        .route("/api/portfolio/summary", get(handlers::portfolio_summary))
        .route("/api/portfolio/pnl-series", get(handlers::pnl_series))
        .route("/api/portfolio/markets-detail", get(handlers::markets_detail))
        .route("/api/portfolio/positions", get(handlers::positions))
        .route("/api/portfolio/positions/{cid}/breakdown", get(handlers::position_breakdown))
        .route("/api/portfolio/orders", get(handlers::open_orders))
        .route("/api/portfolio/history", get(handlers::history))
        .route("/api/portfolio/analysis", get(handlers::analysis))
        .route("/api/portfolio/redemptions", get(handlers::redemptions))
        .route("/api/analysis/maker-behavior", get(handlers::maker_behavior))
        .route("/api/analysis/actions-recent", get(handlers::actions_recent))
        .route("/api/analysis/mid-price-tracking", get(handlers::mid_price_tracking))
        .route("/api/analysis/per-sport", get(handlers::per_sport_breakdown))
        .route("/api/analysis/market-lifecycle/{cid}", get(handlers::market_lifecycle))
        .route("/api/analysis/lifecycle-patterns", get(handlers::lifecycle_patterns))
        .route("/api/analysis/first-leg-context", get(handlers::first_leg_context))
        .route("/api/analysis/quote-timeline/{cid}", get(handlers::quote_timeline))
        .route("/api/analysis/realized-pnl", get(handlers::realized_pnl_breakdown))
        .route("/api/m1/shadow-status", get(handlers::m1_shadow_status))
        .route("/api/diagnostics/coverage", get(handlers::coverage_diagnostics))
        .route("/api/live/summary", get(handlers::live_summary))
        .route("/api/live/state",   get(handlers::trader_state))
        .route("/api/live/orders",  get(handlers::open_orders))
        .route("/api/live/orders/recent", get(handlers::closed_orders))
        .route("/api/live/positions", get(handlers::live_positions))
        .route("/api/live/positions/{cid}/breakdown", get(handlers::live_position_breakdown))
        .route("/api/live/portfolio-summary", get(handlers::live_portfolio_summary))
        .route("/api/live/pnl-series",        get(handlers::live_pnl_series))
        .route("/api/live/markets-upcoming",  get(handlers::live_markets_upcoming))
        .route("/api/system/status", get(handlers::system_status))
        .route("/api/stream", get(handlers::stream))
        .with_state(state)
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = ([0, 0, 0, 0], args.port).into();
    info!(?addr, "rn1-api listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Serialize)]
pub struct ErrorBody { pub error: String }
impl ErrorBody { pub fn new(e: impl ToString) -> (StatusCode, Json<Self>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(Self { error: e.to_string() }))
}}
