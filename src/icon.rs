//! Generated app icon: a filled orange disc on a transparent background.

const ACCENT: [u8; 3] = [0xF5, 0xA6, 0x23];

/// `size`×`size` straight (non-premultiplied) RGBA pixels, row by row: an anti-aliased
/// `#F5A623` disc with a one-pixel margin, transparent elsewhere.
pub fn app_icon_rgba(size: u32) -> Vec<u8> {
    let centre = size as f32 / 2.0;
    let radius = centre - 1.0;
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - centre;
            let dy = y as f32 + 0.5 - centre;
            let coverage = (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            let alpha = (coverage * 255.0).round() as u8;
            if alpha == 0 {
                px.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                px.extend_from_slice(&[ACCENT[0], ACCENT[1], ACCENT[2], alpha]);
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn icon_is_orange_disc() {
        let s = 32; let px = app_icon_rgba(s);
        assert_eq!(px.len(), (s * s * 4) as usize);
        let at = |x: u32, y: u32| &px[((y * s + x) * 4) as usize..][..4];
        assert_eq!(at(16, 16), &[0xF5, 0xA6, 0x23, 0xFF]);
        assert_eq!(at(0, 0)[3], 0);
    }
}
