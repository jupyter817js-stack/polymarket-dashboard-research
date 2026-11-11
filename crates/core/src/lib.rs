//! 공통 도메인 타입 + 상수.
//!
//! 모든 crate 가 의존하는 가장 얕은 레이어. 외부 API 응답 / DB 행 / 내부 메시지가
//! 공통 모양으로 합쳐지는 곳. 부작용 (HTTP / DB) 없음.

pub mod constants;
pub mod money;
pub mod market;
pub mod position;
pub mod order;
pub mod fill;
pub mod event;

pub use constants::*;
pub use money::Usd;
pub use market::{Market, Outcome, FeeSchedule};
pub use position::Position;
pub use order::{Order, OrderSide, OrderStatus};
pub use fill::Fill;
pub use event::EventGroup;
