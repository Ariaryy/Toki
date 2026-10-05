//! The tray icon, drawn rather than shipped. Both trays use it, so it lives above either.

/// An ensō: an open ring with the clock hands crossing its hollow centre and a small accent
/// dot marking the gap where the ring breaks, in premultiplied BGRA.
///
/// The reference artwork is white strokes on a near-black square, but a tray icon has no
/// square behind it — just whatever the taskbar looks like — so the strokes are blue and
/// orange instead of white and red. White would vanish on a light taskbar; a saturated colour
/// reads on both. Sixteen pixels is small enough that the ring and its gap are the whole
/// impression, which is what the four-by-four supersampling is for.
pub fn paint(size: usize) -> Vec<u8> {
    const R: f32 = 37.0;
    const G: f32 = 99.0;
    const B: f32 = 235.0;
    const ACCENT_R: f32 = 242.0;
    const ACCENT_G: f32 = 84.0;
    const ACCENT_B: f32 = 45.0;

    let mut out = vec![0u8; size * size * 4];
    let centre = size as f32 / 2.0;
    let radius = centre - 0.5;
    let thickness = (size as f32 * 0.14).max(1.4);
    let inner_radius = radius - thickness;
    let hand = (size as f32 * 0.075).max(0.7);
    // Half-width, in radians, of the break at the top of the ring where the accent dot sits.
    let gap_half_angle = 12.0f32.to_radians();
    let dot_radius = (thickness * 0.6).max(1.0);
    let dot_centre_y = -(radius - thickness / 2.0);

    for y in 0..size {
        for x in 0..size {
            let mut cover_main = 0.0f32;
            let mut cover_accent = 0.0f32;
            for sub in 0..16 {
                let px = x as f32 + ((sub % 4) as f32 + 0.5) / 4.0 - centre;
                let py = y as f32 + ((sub / 4) as f32 + 0.5) / 4.0 - centre;

                let dot_dy = py - dot_centre_y;
                if px * px + dot_dy * dot_dy <= dot_radius * dot_radius {
                    cover_accent += 1.0 / 16.0;
                    continue;
                }

                let dist_sq = px * px + py * py;
                let in_ring = dist_sq <= radius * radius && dist_sq >= inner_radius * inner_radius;
                // Angle measured from twelve o'clock, wrapped to (-pi, pi] so it stays small
                // near the top regardless of which side it's approached from.
                let from_top = {
                    let raw = py.atan2(px) + std::f32::consts::FRAC_PI_2 + std::f32::consts::PI;
                    raw.rem_euclid(2.0 * std::f32::consts::PI) - std::f32::consts::PI
                };
                let in_gap = in_ring && from_top.abs() <= gap_half_angle;

                let to_twelve = px.abs() <= hand && (-radius * 0.62..=0.0).contains(&py);
                let to_three = py.abs() <= hand && (0.0..=radius * 0.46).contains(&px);

                if (in_ring && !in_gap) || to_twelve || to_three {
                    cover_main += 1.0 / 16.0;
                }
            }
            let total = cover_main + cover_accent;
            if total == 0.0 {
                continue;
            }
            let r = (R * cover_main + ACCENT_R * cover_accent) / total;
            let g = (G * cover_main + ACCENT_G * cover_accent) / total;
            let b = (B * cover_main + ACCENT_B * cover_accent) / total;
            let i = (y * size + x) * 4;
            out[i] = b.round() as u8;
            out[i + 1] = g.round() as u8;
            out[i + 2] = r.round() as u8;
            out[i + 3] = (255.0 * total).round() as u8;
        }
    }
    out
}
