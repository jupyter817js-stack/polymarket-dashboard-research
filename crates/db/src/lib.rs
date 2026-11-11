//! Postgres 풀 + 마이그레이션 + repositories.
//!
//! sqlx 의 macros 는 컴파일 타임 DB 연결을 요구하므로, 본 crate 는 일반 query API
//! 만 사용한다 (offline mode 회피).

use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

pub mod markets;
pub mod fills;
pub mod positions;
pub mod book_events;
pub mod sync;
pub mod trader;

/// 환경변수 `DATABASE_URL` 로부터 풀 생성. 기본 5분 idle, 최대 20 connection.
pub async fn pool_from_env() -> anyhow::Result<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL 환경변수 미설정 — .env 확인"))?;
    pool(&url, 20).await
}

pub async fn pool(url: &str, max: u32) -> anyhow::Result<PgPool> {
    let p = PgPoolOptions::new()
        .max_connections(max)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(3600))
        .connect(url)
        .await?;
    Ok(p)
}

/// 임베디드 마이그레이션 (sqlx::migrate!) 실행.
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}
