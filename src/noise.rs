//! Noise the materials (and the backdrops, and the level's shape) are
//! painted from: a hash, smooth value noise, and Worley cells.

use bevy_math::Vec2;

pub fn noise(x: i32, y: i32) -> u32 {
    let mut value = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263);
    value = (value ^ (value >> 13)).wrapping_mul(1274126177);
    value ^ (value >> 16)
}

/// `x.floor() as i32`, without the call into libm `floor` is on targets
/// without a rounding instruction (the same for every `x` in range).
#[inline(always)]
pub fn floor_i32(x: f32) -> i32 {
    let t = x as i32;
    t - i32::from(x < t as f32)
}

/// Smoothly interpolated lattice noise in [0, 1).
pub fn value_noise(x: f32, y: f32, seed: i32) -> f32 {
    let (x0, y0) = (floor_i32(x), floor_i32(y));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let lattice = |dx: i32, dy: i32| {
        (noise(
            (x0 + dx).wrapping_mul(3) ^ seed.wrapping_mul(0x2F1D),
            y0 + dy,
        ) % 1024) as f32
            / 1024.0
    };
    let (top_left, top_right) = (lattice(0, 0), lattice(1, 0));
    let (bottom_left, bottom_right) = (lattice(0, 1), lattice(1, 1));
    let top = top_left + (top_right - top_left) * sx;
    let bottom = bottom_left + (bottom_right - bottom_left) * sx;
    top + (bottom - top) * sy
}

/// Nearest jittered feature point on a grid of `width` x `height` cells.
pub struct Worley {
    /// Distance to the nearest and second-nearest points: their difference is
    /// small along the boundary between two cells, which draws the cracks.
    pub d1: f32,
    pub d2: f32,
    /// Stable id of the nearest point, for a per-stone tone.
    pub id: u32,
    pub px: f32,
    pub py: f32,
}

/// The feature points around a Worley cell: where each of the nine lies,
/// and its hash.
#[derive(Clone, Copy)]
struct WorleyCell {
    key: (i32, i32, u32, u32, i32),
    points: [(f32, f32, u32); 9],
}

thread_local! {
    /// The cells looked at last, per thread, a few grids at a time: a row
    /// of pixels crosses each cell for a while, so most lookups find their
    /// nine points here instead of hashing them again. The values are the
    /// same either way.
    static WORLEY_CELLS: std::cell::RefCell<[Option<WorleyCell>; 8]> =
        const { std::cell::RefCell::new([None; 8]) };
}

pub fn worley(x: i32, y: i32, width: f32, height: f32, seed: i32) -> Worley {
    let cx = floor_i32(x as f32 / width);
    let cy = floor_i32(y as f32 / height);
    let key = (cx, cy, width.to_bits(), height.to_bits(), seed);
    let points = WORLEY_CELLS.with(|cells| {
        let mut cells = cells.borrow_mut();
        let entry = &mut cells[((seed as u32).wrapping_mul(0x9E37_79B1) >> 29) as usize];
        if let Some(cell) = entry
            && cell.key == key
        {
            return cell.points;
        }
        let mut points = [(0.0, 0.0, 0); 9];
        let mut k = 0;
        for oy in -1..=1 {
            for ox in -1..=1 {
                let (gx, gy) = (cx + ox, cy + oy);
                let hash = noise(gx ^ seed.wrapping_mul(0x51ED), gy.wrapping_mul(31) ^ seed);
                let px = (gx as f32 + 0.15 + (hash % 1000) as f32 / 1000.0 * 0.7) * width;
                let py = (gy as f32 + 0.15 + ((hash >> 10) % 1000) as f32 / 1000.0 * 0.7) * height;
                points[k] = (px, py, hash);
                k += 1;
            }
        }
        *entry = Some(WorleyCell { key, points });
        points
    });
    let mut best = Worley {
        d1: f32::MAX,
        d2: f32::MAX,
        id: 0,
        px: 0.0,
        py: 0.0,
    };
    for (px, py, hash) in points {
        let d = Vec2::new(x as f32 + 0.5 - px, y as f32 + 0.5 - py).length();
        if d < best.d1 {
            best.d2 = best.d1;
            best.d1 = d;
            best.id = hash;
            best.px = px;
            best.py = py;
        } else if d < best.d2 {
            best.d2 = d;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_i32_is_floor() {
        let mut x = -300.0f32;
        while x < 300.0 {
            assert_eq!(floor_i32(x), x.floor() as i32, "{x}");
            x += 0.037;
        }
        for x in [
            -1.0,
            -0.0,
            0.0,
            1.0,
            -1e-7,
            1e-7,
            16_777_216.0,
            -16_777_216.0,
        ] {
            assert_eq!(floor_i32(x), x.floor() as i32, "{x}");
        }
    }
}
