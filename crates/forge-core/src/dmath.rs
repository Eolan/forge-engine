//! Deterministic floating-point math.
//!
//! Platform math libraries (the MSVC runtime, glibc…) round transcendental functions
//! differently, so `f64::sin` can return different bits on Windows and Linux. Anything that
//! must match across machines — generation and simulation — uses these functions instead:
//! they run the pure-Rust `libm` code, which is identical everywhere.
//!
//! Operations that IEEE 754 specifies exactly are safe to use directly: `+ - * /`, `%`,
//! `sqrt`, `mul_add`, `abs`, `floor`, `ceil`, `round`, `trunc`, `min`, `max`.
//! (See Fiedler, "Floating Point Determinism", `docs/RESEARCH.md` §3.)

use std::ops::{Div, Mul, MulAssign};

mod sealed {
    pub trait Sealed {}
    impl Sealed for f32 {}
    impl Sealed for f64 {}
}

macro_rules! deterministic_functions {
    (
        unary: [$($unary:ident => $unary_f64:ident, $unary_f32:ident;)*]
        binary: [$($binary:ident => $binary_f64:ident, $binary_f32:ident;)*]
    ) => {
        /// Floating-point types supported by these functions: `f32` and `f64`.
        pub trait Real:
            Copy + Mul<Output = Self> + MulAssign + Div<Output = Self> + sealed::Sealed
        {
            /// The value 1.
            const ONE: Self;
            $(#[doc(hidden)] fn $unary(self) -> Self;)*
            $(#[doc(hidden)] fn $binary(self, other: Self) -> Self;)*
        }

        impl Real for f64 {
            const ONE: Self = 1.0;
            $(#[inline] fn $unary(self) -> Self { libm::$unary_f64(self) })*
            $(#[inline] fn $binary(self, other: Self) -> Self { libm::$binary_f64(self, other) })*
        }

        impl Real for f32 {
            const ONE: Self = 1.0;
            $(#[inline] fn $unary(self) -> Self { libm::$unary_f32(self) })*
            $(#[inline] fn $binary(self, other: Self) -> Self { libm::$binary_f32(self, other) })*
        }

        $(
            #[doc = concat!("Deterministic `", stringify!($unary), "`, identical on every platform.")]
            #[inline]
            pub fn $unary<T: Real>(x: T) -> T {
                <T as Real>::$unary(x)
            }
        )*

        $(
            #[doc = concat!("Deterministic `", stringify!($binary), "`, identical on every platform.")]
            #[inline]
            pub fn $binary<T: Real>(x: T, y: T) -> T {
                <T as Real>::$binary(x, y)
            }
        )*
    };
}

deterministic_functions! {
    unary: [
        sin => sin, sinf;
        cos => cos, cosf;
        tan => tan, tanf;
        asin => asin, asinf;
        acos => acos, acosf;
        atan => atan, atanf;
        sinh => sinh, sinhf;
        cosh => cosh, coshf;
        tanh => tanh, tanhf;
        exp => exp, expf;
        exp2 => exp2, exp2f;
        ln => log, logf;
        log2 => log2, log2f;
        log10 => log10, log10f;
        cbrt => cbrt, cbrtf;
    ]
    binary: [
        atan2 => atan2, atan2f;
        powf => pow, powf;
        hypot => hypot, hypotf;
    ]
}

/// Deterministic `sin` and `cos` of the same angle.
#[inline]
pub fn sin_cos<T: Real>(x: T) -> (T, T) {
    (sin(x), cos(x))
}

/// Deterministic integer power by repeated squaring (exact for small exponents).
#[inline]
pub fn powi<T: Real>(mut base: T, mut exponent: i32) -> T {
    let invert = exponent < 0;
    exponent = exponent.abs();
    let mut result = T::ONE;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result *= base;
        }
        base *= base;
        exponent >>= 1;
    }
    if invert { T::ONE / result } else { result }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_values() {
        assert!((sin(0.5_f64) - 0.479_425_538_604_203).abs() < 1e-15);
        assert!((sin(0.5_f32) - 0.479_425_55).abs() < 1e-6);
        assert!((powf(2.0_f64, 10.0) - 1024.0).abs() < 1e-12);
        assert_eq!(powi(2.0_f64, 10), 1024.0);
        assert_eq!(powi(2.0_f32, -2), 0.25);
    }
}
