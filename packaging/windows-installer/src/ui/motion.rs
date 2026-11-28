// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/motion.rs
// Responsibility:   Easing functions of the design: the curve of the interface and the curves of the splash.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Easing functions of the design.
//!
//! The interface uses `cubic-bezier(0.2, 0.7, 0.2, 1)` (section 4 of the design); the splash uses
//! the curves of the table of section 7.2.

/// The part of `value` between `from` and `to`, limited to 0 and 1.
pub(crate) fn norm(value: f32, from: f32, to: f32) -> f32 {
    ((value - from) / (to - from)).clamp(0.0, 1.0)
}

/// Linear interpolation.
pub(crate) fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

/// `1 - (1 - x)^3`.
pub(crate) fn out_cubic(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}

/// Cubic in and out.
pub(crate) fn in_out_cubic(x: f32) -> f32 {
    if x < 0.5 {
        4.0 * x * x * x
    } else {
        1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
    }
}

/// Cubic out with a slight overshoot.
pub(crate) fn out_back(x: f32) -> f32 {
    let c1 = 1.1;
    let c3 = c1 + 1.0;
    1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
}

/// `x * x * (3 - 2x)`.
pub(crate) fn smoothstep(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

/// The curve `cubic-bezier(x1, y1, x2, y2)` of CSS at the time `x` (0 to 1).
pub(crate) fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let sample = |t: f32, a: f32, b: f32| {
        3.0 * (1.0 - t).powi(2) * t * a + 3.0 * (1.0 - t) * t * t * b + t.powi(3)
    };
    let mut low = 0.0f32;
    let mut high = 1.0f32;
    let mut t = x;
    for _ in 0..24 {
        let value = sample(t, x1, x2);
        if (value - x).abs() < 1e-5 {
            break;
        }
        if value < x {
            low = t
        } else {
            high = t
        }
        t = (low + high) / 2.0;
    }
    sample(t, y1, y2)
}

/// The curve of the interface: `cubic-bezier(0.2, 0.7, 0.2, 1)`.
pub(crate) fn interface(x: f32) -> f32 {
    bezier(0.2, 0.7, 0.2, 1.0, x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_and_end_where_they_should() {
        assert!(interface(0.0).abs() < 1e-3 && (interface(1.0) - 1.0).abs() < 1e-3);
        assert!(interface(0.5) > 0.5);
        assert!(out_back(0.0).abs() < 1e-3 && (out_back(1.0) - 1.0).abs() < 1e-3);
        assert!((smoothstep(0.5) - 0.5).abs() < 1e-6);
        assert!((in_out_cubic(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn norm_is_clamped() {
        assert_eq!(norm(-1.0, 0.0, 2.0), 0.0);
        assert_eq!(norm(1.0, 0.0, 2.0), 0.5);
        assert_eq!(norm(5.0, 0.0, 2.0), 1.0);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
