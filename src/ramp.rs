use crate::config::Game;

/// R, G, B rows of 256 entries; same layout `SetDeviceGammaRamp` expects.
pub type GammaRamp = [[u16; 256]; 3];

#[cfg(test)]
pub fn identity() -> GammaRamp {
    let mut r = [[0u16; 256]; 3];
    for ch in r.iter_mut() {
        for (i, v) in ch.iter_mut().enumerate() {
            *v = (i * 257) as u16;
        }
    }
    r
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveParams {
    pub gamma: f64,
    pub contrast: f64,
    pub shadow_boost: f64,
}

impl CurveParams {
    #[cfg(any(test, debug_assertions))]
    pub const NEUTRAL: CurveParams = CurveParams { gamma: 1.0, contrast: 1.0, shadow_boost: 0.0 };

    /// Switched-off controls contribute their neutral value.
    pub fn from_game(g: &Game) -> CurveParams {
        let pick = |c: &crate::config::Control, neutral: f64| if c.enabled { c.value } else { neutral };
        CurveParams {
            gamma: pick(&g.gamma, 1.0),
            contrast: pick(&g.contrast, 1.0),
            shadow_boost: pick(&g.shadow_boost, 0.0),
        }
    }
}

/// Maps input level `x` in 0..=1 to output level in 0..=1.
pub fn curve(p: &CurveParams, x: f64) -> f64 {
    let mut y = x.powf(1.0 / p.gamma);
    let s = 2.0 * p.shadow_boost / 100.0; // <= 2 keeps the curve monotonic
    y += s * y * (1.0 - y) * (1.0 - y);
    y = (y - 0.5) * p.contrast + 0.5;
    y.clamp(0.0, 1.0)
}

/// Applies the curve on top of the display's normal ramp, interpolating each channel.
pub fn layered(normal: &GammaRamp, p: &CurveParams) -> GammaRamp {
    let mut out = [[0u16; 256]; 3];
    for i in 0..256 {
        let mut pos = curve(p, i as f64 / 255.0) * 255.0;
        if (pos - pos.round()).abs() < 1e-6 {
            pos = pos.round();
        }
        let lo = pos.floor() as usize;
        let hi = (lo + 1).min(255);
        let t = pos - lo as f64;
        for ch in 0..3 {
            let a = normal[ch][lo] as f64;
            let b = normal[ch][hi] as f64;
            out[ch][i] = (a + (b - a) * t).round().clamp(0.0, 65535.0) as u16;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(g: f64, c: f64, s: f64) -> CurveParams { CurveParams { gamma: g, contrast: c, shadow_boost: s } }
    #[test] fn identity_endpoints() { let r = identity(); assert_eq!((r[0][0], r[1][128], r[2][255]), (0, 128 * 257, 65535)); }
    #[test] fn neutral_curve_is_identity() { for i in 0..=10 { let x = i as f64 / 10.0; assert!((curve(&CurveParams::NEUTRAL, x) - x).abs() < 1e-12); } }
    #[test] fn gamma_two_brightens_quarter_to_half() { assert!((curve(&p(2.0, 1.0, 0.0), 0.25) - 0.5).abs() < 1e-9); }
    #[test] fn contrast_two_scales_around_mid() {
        let q = p(1.0, 2.0, 0.0);
        assert!((curve(&q, 0.75) - 1.0).abs() < 1e-9); assert!(curve(&q, 0.25).abs() < 1e-9); assert!((curve(&q, 0.6) - 0.7).abs() < 1e-9);
    }
    #[test] fn shadow_boost_lifts_darks_keeps_endpoints() {
        let q = p(1.0, 1.0, 100.0);
        assert!((curve(&q, 0.05) - 0.14025).abs() < 1e-9); assert_eq!(curve(&q, 0.0), 0.0); assert!((curve(&q, 1.0) - 1.0).abs() < 1e-12);
    }
    #[test] fn curve_monotonic_and_bounded_at_extremes() {
        for g in [0.5, 3.0] { for c in [0.5, 2.0] { for s in [0.0, 100.0] {
            let mut prev = -1.0;
            for i in 0..256 { let y = curve(&p(g, c, s), i as f64 / 255.0); assert!((0.0..=1.0).contains(&y) && y >= prev); prev = y; }
        }}}
    }
    fn odd_normal() -> GammaRamp {
        let mut r = [[0u16; 256]; 3];
        for i in 0..256 { r[0][i] = (i * 257) as u16; r[1][i] = (i * 200) as u16; r[2][i] = 30000; }
        r
    }
    #[test] fn layered_neutral_returns_normal_exactly() { assert_eq!(layered(&odd_normal(), &CurveParams::NEUTRAL), odd_normal()); }
    #[test] fn layered_gamma_on_identity() { let r = layered(&identity(), &p(2.0, 1.0, 0.0)); assert!((r[0][64] as i32 - 32832).abs() <= 1); }
    #[test] fn layered_follows_each_normal_channel() { let r = layered(&odd_normal(), &p(2.0, 1.0, 0.0)); assert!(r[2].iter().all(|&v| v == 30000)); }
    #[test] fn from_game_uses_only_switched_on() {
        let mut g = crate::config::Game::new("a.exe");
        g.gamma = crate::config::Control { enabled: true, value: 2.0 };
        g.contrast = crate::config::Control { enabled: false, value: 1.8 };
        assert_eq!(CurveParams::from_game(&g), p(2.0, 1.0, 0.0));
    }
}
