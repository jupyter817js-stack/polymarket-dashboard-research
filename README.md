# Polymarket dashboard and backend research

A Next.js/TypeScript frontend with a Rust workspace for market-data ingestion, APIs, positions, signing and trading-related processing.

## Structure

| Directory | Responsibility |
| --- | --- |
| apps/web | Next.js application and data views |
| apps/api | axum API handlers and PostgreSQL access |
| apps/lifecycle-collector | Collection and lifecycle processing |
| apps/trader | Trading workflow implementation for review |
| crates/clob-client | CLOB request client |
| crates/signer | Authentication and order-signing routines |
| crates/ws-client | WebSocket handling |
| crates/planner | Planning logic |
| crates/db | Query and persistence layer |
| migrations | Database schema evolution |

## Frontend review

~~~sh
cd apps/web
npm ci --ignore-scripts
npm run typecheck
npm run build
~~~

The development server uses port 3030. Its API rewrite defaults to localhost:8000; configure RN1_API_BASE for a different backend. There is no embedded production database or account. Database-backed screens require a separately configured local backend and sample data.

## Rust source review

The workspace declares Rust 1.85 or later. Start with apps/api/src/main.rs, crates/planner, crates/signer, and the database migrations. Build checking is performed with cargo check --workspace where the required toolchain is available. No live trading process is required to inspect the source.

## Boundaries

The public snapshot contains implementation code, not a claim that every service is deployed or all trading behavior has been validated. Signing and execution routines require separate security review. See VALIDATION.md for what was actually checked for this edition.

## About this public edition

This is a source snapshot prepared for code review. Operational logs, account-specific configuration, runtime state, private keys, and the private repository history are excluded. The original implementation is preserved separately. This snapshot is not evidence of audited production readiness or investment performance.

Please use Upwork Messages for project discussions.
