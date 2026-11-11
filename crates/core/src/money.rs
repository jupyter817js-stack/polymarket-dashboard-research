//! USD wrapper — Decimal 기반, JSON 직렬화 시 문자열로 보존되어 정밀도 손실 없음.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Sub, SubAssign, Mul};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Usd(pub Decimal);

impl Usd {
    pub const ZERO: Self = Self(Decimal::ZERO);

    pub fn from_f64(v: f64) -> Self {
        Self(Decimal::try_from(v).unwrap_or_default())
    }

    pub fn as_f64(self) -> f64 {
        self.0.try_into().unwrap_or(0.0)
    }
}

impl Add for Usd { type Output = Self; fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0) } }
impl Sub for Usd { type Output = Self; fn sub(self, rhs: Self) -> Self { Self(self.0 - rhs.0) } }
impl AddAssign for Usd { fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0; } }
impl SubAssign for Usd { fn sub_assign(&mut self, rhs: Self) { self.0 -= rhs.0; } }
impl Mul<Decimal> for Usd { type Output = Self; fn mul(self, rhs: Decimal) -> Self { Self(self.0 * rhs) } }
