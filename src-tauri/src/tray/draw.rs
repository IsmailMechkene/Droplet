//! Draws a tray icon: a usage ring, the percentage in a built-in bitmap font,
//! and the pace tick. Same colours as the notch rings. Output is RGBA.

use std::f64::consts::PI;

use super::font::{Font, LARGE, SMALL};
use crate::config::{Appearance, ColorMode};

/// What one tray ring shows.
#[derive(Debug, Clone, PartialEq)]
pub struct RingIcon {
    /// Main limit, 0..=100.
    pub pct: f64,
    /// Even pace, 0..=1.
    pub pace: Option<f64>,
    pub stale: bool,
    /// Draw for a light taskbar (dark ink) or a dark one (light ink).
    pub light_taskbar: bool,
}

pub type Rgba = [f64; 4];

/// The tray sizes Windows uses at 100 %, 125 %, 150 % and 200 % scaling.
pub const SIZES: [u32; 4] = [16, 20, 24, 32];

/// Picks the drawn size closest to (and not smaller than) what Windows asks for.
pub fn size_for(wanted: u32) -> u32 {
    SIZES.into_iter().find(|s| *s >= wanted).unwrap_or(32)
}

/// OKLCH → gamma-encoded sRGB, components 0..=1 (out-of-gamut values are clamped).
pub fn oklch(l: f64, c: f64, h: f64) -> [f64; 3] {
    let (a, b) = (c * (h * PI / 180.0).cos(), c * (h * PI / 180.0).sin());
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    let linear = [
        4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
        -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3,
    ];
    linear.map(|v| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    })
}

/// The usage colour, matching `usageColor` in ui/shared/ring.js.
pub fn usage_color(pct: f64, look: &Appearance) -> [f64; 3] {
    const GREEN: [f64; 3] = [0.70, 0.15, 152.0];
    const AMBER: [f64; 3] = [0.78, 0.15, 75.0];
    const RED: [f64; 3] = [0.64, 0.20, 27.0];
    let mix = |a: [f64; 3], b: [f64; 3], t: f64| {
        let t = t.clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };
    let (warn, crit) = (look.warn_at as f64, look.crit_at as f64);
    let c = match look.color_mode {
        ColorMode::Smooth if pct <= warn - 20.0 => GREEN,
        ColorMode::Smooth if pct < warn => mix(GREEN, AMBER, (pct - (warn - 20.0)) / 20.0),
        ColorMode::Smooth if pct < crit => mix(AMBER, RED, (pct - warn) / (crit - warn).max(1.0)),
        ColorMode::Smooth => RED,
        ColorMode::Hard if pct >= crit => RED,
        ColorMode::Hard if pct >= warn => AMBER,
        ColorMode::Hard => GREEN,
    };
    oklch(c[0], c[1], c[2])
}

/// Source-over blend of a colour with `alpha` coverage onto a premultiplied canvas pixel
/// (colour channels hold colour × alpha).
fn blend(px: &mut Rgba, rgb: [f64; 3], alpha: f64) {
    if alpha <= 0.0 {
        return;
    }
    for i in 0..3 {
        px[i] = rgb[i] * alpha + px[i] * (1.0 - alpha);
    }
    px[3] = alpha + px[3] * (1.0 - alpha);
}

/// The text inside the ring: 0..=99, or "!" when the limit is used up.
pub fn label(pct: f64) -> String {
    let rounded = pct.round().clamp(0.0, 100.0) as u32;
    if rounded >= 100 { "!".into() } else { rounded.to_string() }
}

/// Renders the icon at `size`×`size`, returning straight (not premultiplied) RGBA bytes.
pub fn render(icon: &RingIcon, size: u32, look: &Appearance) -> Vec<u8> {
    let n = size as usize;
    let mut canvas = vec![[0.0f64; 4]; n * n];
    let s = size as f64;
    let centre = s / 2.0;
    let stroke = (s / 8.0).max(2.0);
    let radius = centre - stroke / 2.0 - 0.25;

    let ink = if icon.light_taskbar { [0.10, 0.11, 0.12] } else { [0.96, 0.96, 0.97] };
    let track = if icon.light_taskbar { ([0.0, 0.0, 0.0], 0.16) } else { ([1.0, 1.0, 1.0], 0.24) };
    let arc_rgb = if icon.stale { [0.62, 0.63, 0.66] } else { usage_color(icon.pct, look) };
    let progress = (icon.pct / 100.0).clamp(0.0, 1.0);

    // Ring: 4×4 supersampling for smooth edges.
    const SS: usize = 4;
    for y in 0..n {
        for x in 0..n {
            let (mut on_track, mut on_arc) = (0usize, 0usize);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f64 + (sx as f64 + 0.5) / SS as f64 - centre;
                    let py = y as f64 + (sy as f64 + 0.5) / SS as f64 - centre;
                    if ((px * px + py * py).sqrt() - radius).abs() > stroke / 2.0 {
                        continue;
                    }
                    // Angle clockwise from 12 o'clock, 0..1.
                    let turn = (px.atan2(-py) / (2.0 * PI)).rem_euclid(1.0);
                    if turn < progress { on_arc += 1 } else { on_track += 1 }
                }
            }
            let px = &mut canvas[y * n + x];
            blend(px, track.0, track.1 * on_track as f64 / (SS * SS) as f64);
            blend(px, arc_rgb, on_arc as f64 / (SS * SS) as f64);
        }
    }

    // Pace tick: a short radial mark across the ring where an even pace would be.
    if let (Some(pace), false) = (icon.pace, icon.stale) {
        let angle = pace.clamp(0.0, 1.0) * 2.0 * PI;
        let (dx, dy) = (angle.sin(), -angle.cos());
        // About 1 px wide (2 px at 32), across the ring band, anti-aliased by supersampling.
        let half = (s / 32.0).max(0.5);
        for y in 0..n {
            for x in 0..n {
                let mut hits = 0usize;
                for sy in 0..SS {
                    for sx in 0..SS {
                        let px = x as f64 + (sx as f64 + 0.5) / SS as f64 - centre;
                        let py = y as f64 + (sy as f64 + 0.5) / SS as f64 - centre;
                        let along = px * dx + py * dy;
                        let across = (px * dy - py * dx).abs();
                        if across <= half && (along - radius).abs() <= stroke / 2.0 + 0.5 {
                            hits += 1;
                        }
                    }
                }
                blend(&mut canvas[y * n + x], ink, hits as f64 / (SS * SS) as f64);
            }
        }
    }

    // Digits, centred, pixel-exact.
    let text = label(icon.pct);
    let font: &Font = if size >= 24 { &LARGE } else { &SMALL };
    let bold = size >= 32;
    let glyph_w = font.width + u32::from(bold);
    let text_w = text.chars().count() as u32 * glyph_w + text.chars().count().saturating_sub(1) as u32;
    let x0 = (size.saturating_sub(text_w) + 1) / 2;
    let y0 = (size.saturating_sub(font.height) + 1) / 2;
    let digit_ink = if icon.pct >= 100.0 && !icon.stale { usage_color(100.0, look) } else { ink };
    for (i, c) in text.chars().enumerate() {
        let gx = x0 + i as u32 * (glyph_w + 1);
        for y in 0..font.height {
            for x in 0..font.width {
                if font.pixel(c, x, y) {
                    for b in 0..=u32::from(bold) {
                        let (cx, cy) = ((gx + x + b) as usize, (y0 + y) as usize);
                        if cx < n && cy < n {
                            blend(&mut canvas[cy * n + cx], digit_ink, 1.0);
                        }
                    }
                }
            }
        }
    }

    // The canvas is premultiplied; Windows icons want straight alpha.
    canvas
        .iter()
        .flat_map(|p| {
            let a = p[3];
            let straight = |c: f64| if a > 0.0 { (c / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            [straight(p[0]), straight(p[1]), straight(p[2]), (a * 255.0).round() as u8]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(pct: f64) -> RingIcon {
        RingIcon { pct, pace: Some(0.5), stale: false, light_taskbar: false }
    }

    #[test]
    fn labels() {
        assert_eq!(label(0.0), "0");
        assert_eq!(label(7.4), "7");
        assert_eq!(label(64.5), "65");
        assert_eq!(label(99.4), "99");
        assert_eq!(label(99.6), "!");
        assert_eq!(label(100.0), "!");
    }

    #[test]
    fn sizes_by_dpi() {
        assert_eq!(size_for(16), 16);
        assert_eq!(size_for(18), 20);
        assert_eq!(size_for(24), 24);
        assert_eq!(size_for(30), 32);
        assert_eq!(size_for(48), 32);
    }

    #[test]
    fn oklch_matches_known_colours() {
        let white = oklch(1.0, 0.0, 0.0);
        assert!(white.iter().all(|c| (c - 1.0).abs() < 0.01));
        let black = oklch(0.0, 0.0, 0.0);
        assert!(black.iter().all(|c| c.abs() < 0.01));
        // The notch's green is clearly green.
        let g = oklch(0.70, 0.15, 152.0);
        assert!(g[1] > g[0] && g[1] > g[2]);
        let r = oklch(0.64, 0.20, 27.0);
        assert!(r[0] > r[1] && r[0] > r[2]);
    }

    #[test]
    fn colour_levels_follow_settings() {
        let look = Appearance::default();
        let green = usage_color(10.0, &look);
        let amber = usage_color(75.0, &look);
        let red = usage_color(95.0, &look);
        assert!(green[1] > green[0]);
        assert!(amber[0] > amber[2] && amber[1] > amber[2]);
        assert!(red[0] > red[1]);
        assert_eq!(usage_color(69.0, &look), green, "hard step stays green below warning");
        let smooth = Appearance { color_mode: ColorMode::Smooth, ..Appearance::default() };
        assert_ne!(usage_color(60.0, &smooth), green, "smooth ramp blends before warning");
    }

    #[test]
    fn renders_every_size_with_transparent_corners() {
        let look = Appearance::default();
        for size in SIZES {
            let px = render(&icon(64.0), size, &look);
            assert_eq!(px.len(), (size * size * 4) as usize);
            assert_eq!(px[3], 0, "top-left corner is transparent at {size}px");
            let opaque = px.chunks(4).filter(|p| p[3] > 200).count();
            assert!(opaque > (size * size / 6) as usize, "ring and digits are drawn at {size}px");
        }
    }

    #[test]
    fn the_arc_grows_with_usage() {
        let look = Appearance::default();
        let coloured = |pct| {
            render(&RingIcon { pace: None, ..icon(pct) }, 32, &look)
                .chunks(4)
                .filter(|p| p[3] > 200 && i32::from(p[1]) > i32::from(p[0]) + 40) // green, opaque
                .count()
        };
        assert!(coloured(10.0) < coloured(40.0));
        assert!(coloured(0.0) == 0);
    }
}

/// Opt-in: writes sample icons as raw RGBA for a visual check.
/// `DROPLET_TRAY_OUT=<dir> cargo test tray_samples -- --ignored`
#[cfg(test)]
mod samples {
    use super::*;

    #[test]
    #[ignore = "writes files for a visual check"]
    fn tray_samples() {
        let dir = std::path::PathBuf::from(std::env::var_os("DROPLET_TRAY_OUT").expect("DROPLET_TRAY_OUT"));
        let look = Appearance::default();
        for (name, pct, light) in [("ok", 33.0, false), ("warn", 76.0, false), ("crit", 94.0, false), ("full", 100.0, false), ("light", 64.0, true)] {
            for size in SIZES {
                let icon = RingIcon { pct, pace: Some(0.45), stale: false, light_taskbar: light };
                std::fs::write(dir.join(format!("{name}-{size}.rgba")), render(&icon, size, &look)).unwrap();
            }
        }
    }
}
