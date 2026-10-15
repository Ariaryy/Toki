//! The same artwork as the application, decoded at build time and embedded as RGBA.

include!(concat!(env!("OUT_DIR"), "/tray-icon-size.rs"));
const SOURCE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tray-icon.rgba"));

/// Scale the shared artwork to the tray host's requested size, returning straight BGRA.
/// Area filtering averages premultiplied colours so transparent edges do not grow halos.
/// Windows premultiplies this output for GDI; Linux reorders it to the protocol's ARGB.
pub fn paint(size: usize) -> Vec<u8> {
    let mut out = vec![0; size * size * 4];
    if size == 0 {
        return out;
    }
    let scale = SOURCE_SIZE as f32 / size as f32;
    for y in 0..size {
        for x in 0..size {
            let (left, top) = (x as f32 * scale, y as f32 * scale);
            let (right, bottom) = ((x + 1) as f32 * scale, (y + 1) as f32 * scale);
            let mut rgba = [0.0f32; 4];
            for sy in top.floor() as usize..(bottom.ceil() as usize).min(SOURCE_SIZE) {
                for sx in left.floor() as usize..(right.ceil() as usize).min(SOURCE_SIZE) {
                    let weight = (right.min((sx + 1) as f32) - left.max(sx as f32))
                        * (bottom.min((sy + 1) as f32) - top.max(sy as f32));
                    let p = &SOURCE[(sy * SOURCE_SIZE + sx) * 4..][..4];
                    let alpha = p[3] as f32 / 255.0;
                    for c in 0..3 {
                        rgba[c] += p[c] as f32 * alpha * weight;
                    }
                    rgba[3] += p[3] as f32 * weight;
                }
            }
            let i = (y * size + x) * 4;
            if rgba[3] > 0.0 {
                for c in 0..3 {
                    out[i + 2 - c] = (rgba[c] * 255.0 / rgba[3]).round() as u8;
                }
                out[i + 3] = (rgba[3] / (scale * scale)).round() as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_size_preserves_the_shared_artwork() {
        for (actual, source) in paint(SOURCE_SIZE)
            .chunks_exact(4)
            .zip(SOURCE.chunks_exact(4))
        {
            assert_eq!(actual[3], source[3]);
            if source[3] != 0 {
                assert_eq!(&actual[..3], &[source[2], source[1], source[0]]);
            }
        }
    }
    #[test]
    fn tray_sizes_keep_the_transparent_surround_and_opaque_background() {
        for size in [16, 20, 22, 24, 32, 48] {
            let pixels = paint(size);
            assert_eq!(pixels.len(), size * size * 4);
            assert_eq!(pixels[3], 0);
            assert_eq!(pixels[(size / 2 * size + size / 2) * 4 + 3], 255);
        }
        assert!(paint(0).is_empty());
    }
}
