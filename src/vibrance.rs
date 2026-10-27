#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VibranceInfo {
    pub current: i32,
    pub min: i32,
    pub max: i32,
    pub default: i32,
}

/// Maps the UI value (0-100, neutral 50) onto the driver's raw range.
pub fn ui_to_raw(ui: f64, info: &VibranceInfo) -> i32 {
    let ui = ui.clamp(0.0, 100.0);
    let (min, def, max) = (info.min as f64, info.default as f64, info.max as f64);
    let raw = if ui <= 50.0 {
        min + (def - min) * (ui / 50.0)
    } else {
        def + (max - def) * ((ui - 50.0) / 50.0)
    };
    raw.round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn maps_positive_only_range() {
        let i = VibranceInfo { current: 0, min: 0, max: 63, default: 0 };
        assert_eq!([0.0, 50.0, 75.0, 100.0, 120.0, -5.0].map(|u| ui_to_raw(u, &i)), [0, 0, 32, 63, 63, 0]);
    }
    #[test] fn maps_symmetric_range() {
        let i = VibranceInfo { current: 0, min: -50, max: 50, default: 0 };
        assert_eq!([0.0, 25.0, 50.0, 100.0].map(|u| ui_to_raw(u, &i)), [-50, -25, 0, 50]);
    }
}
