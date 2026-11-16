//! How each material looks: its texture pixel by pixel (from noise, so it
//! never has to be stored), the optics of liquids, and the crust along
//! exposed faces of rock.

use super::*;

/// How a liquid looks: what shows through it, how it deepens, how glassy
/// its surface is and whether it glows. Lava also shades its molten body
/// from the live shape of the pool.
#[derive(Clone, Copy, Debug)]
pub struct Optics {
    /// Coverage (0..=255) of a pixel at the surface and at `LIQUID_DEPTH`.
    pub alpha: (f32, f32),
    /// Share of each color channel left at full depth: what it absorbs.
    pub depth_tint: [f32; 3],
    /// Brightening per cell/tick of motion, so moving liquid churns.
    pub churn: [f32; 3],
    /// A drop in the air: how much lighter, and its coverage.
    pub drop: (f32, u8),
    /// How much neighboring pigment blends into the body of a pool. Airborne
    /// drops still keep their individual carried grain.
    pub body_smooth: f32,
    /// Sparse moving light along the exposed surface, in unlit albedo space.
    pub surface_glint: [f32; 3],
    pub glint_strength: f32,
    /// How strongly its surface mirrors what is above (0..=1).
    pub reflect: f32,
    /// How much it bends what is seen through it (0..=1).
    pub refract: f32,
    /// Light it gives off itself (0..=255 per channel), and the share of
    /// that which lights its surroundings (0 glows without lighting).
    pub glow: [f32; 3],
    pub glow_light: f32,
    /// White water: how far fast-moving liquid near air (where a pour lands,
    /// a wave breaks, a bank collapses) whitens toward its surface glint.
    pub foam: f32,
}

/// Sparse lighter flecks, one or two pixels across, like mineral grains
/// catching the light: `lift` brighter on roughly one cell in `odds`.
pub(super) fn speck(x: i32, y: i32, odds: u32, lift: i16) -> i16 {
    let seed = noise(x.div_euclid(2) ^ 0x51ec, y);
    if seed.is_multiple_of(odds) && (x.rem_euclid(2) == 0 || seed & 64 == 0) {
        lift + (seed >> 8) as i16 % 7
    } else {
        0
    }
}

/// Darkening for a pixel `edge` cells from a cell boundary: a hard line and a
/// softer shoulder beside it.
pub(super) fn crack_shade(edge: f32, line: i16, shoulder: i16) -> i16 {
    if edge < 1.0 {
        -line
    } else if edge < 2.0 {
        -shoulder
    } else {
        0
    }
}

impl TerrainKind {
    /// The pixel's own shade, anchored to where it was created. Solids get a
    /// structured texture (cobbles, planks, pebbles) rather than per-pixel
    /// static, so terrain reads as material at a glance; moving material keeps
    /// the shade it was given.
    pub fn color(self, x: i32, y: i32) -> [u8; 4] {
        let fine = (noise(x, y) % 15) as i16 - 7;
        let coarse = (noise(x.div_euclid(5), y.div_euclid(3)) % 13) as i16 - 6;
        let strata = ((y as f32 * 0.7 + (x as f32 * 0.13).sin() * 2.0).sin() * 6.0) as i16;
        let (base, variation, alpha): ([i16; 3], [i16; 3], u8) = match self {
            Self::Soil => {
                // Loam with darker clumps, root-dark pockets, and scattered pebbles.
                let blob = (value_noise(x as f32 / 7.0, y as f32 / 4.0, 3) * 22.0) as i16 - 11;
                let clump = if value_noise(x as f32 / 3.0, y as f32 / 3.0, 9) > 0.74 {
                    -12
                } else {
                    0
                };
                let v = fine / 2 + blob + clump + strata / 2;
                if noise(x, y ^ 0x5eed).is_multiple_of(37) {
                    ([128, 112, 96], [fine; 3], 255)
                } else {
                    ([100, 74, 54], [v, v - 1, v - 2], 255)
                }
            }
            Self::Stone => {
                // Noita-style cave rock: a dark, cool base mottled in large
                // soft patches, with scattered lighter flecks and the odd
                // broken seam. Exposed faces get a pebbly crust on top of
                // this (see `edge_crust`).
                // Packed stones, Noita's rock: each a few cells across with
                // its own tone, lit on one side and shadowed on the other,
                // dark cracks between, so the rock reads at a glance even
                // in dim light.
                let broad = (value_noise(x as f32 / 13.0, y as f32 / 9.0, 11) * 10.0) as i16 - 5;
                // Big slabs in places, small pebbles in others, and the
                // cracks between them broken, not a net.
                let stone = if value_noise(x as f32 / 23.0, y as f32 / 17.0, 21) > 0.55 {
                    worley(x, y, 10.0, 6.5, 14)
                } else {
                    worley(x, y, 5.5, 4.0, 13)
                };
                let tone = (stone.id % 13) as i16 - 6;
                let gap = stone.d2 - stone.d1;
                let open = value_noise(x as f32 / 4.0, y as f32 / 4.0, 19) > 0.38;
                let crack = if gap < 0.85 && open {
                    -24
                } else if gap < 1.6 {
                    -6
                } else {
                    0
                };
                let lit = ((stone.py - y as f32) * 1.8 - (x as f32 - stone.px) * 1.2)
                    .clamp(-9.0, 10.0) as i16;
                let fleck = speck(x, y, 13, 20);
                // A slow drift between cooler and warmer rock.
                let warm = (value_noise(x as f32 / 29.0, y as f32 / 17.0, 15) * 12.0) as i16 - 6;
                let v = broad + tone + crack + lit + fleck + fine / 5 + strata / 6;
                (
                    [62 + warm, 59 + warm / 2, 58 - warm / 2],
                    [v, v * 15 / 16, v * 7 / 8 + 2],
                    255,
                )
            }
            Self::Bedrock => {
                let slab = worley(x, y, 16.0, 10.0, 23);
                let tone = (slab.id % 7) as i16 - 3;
                let crack = crack_shade(slab.d2 - slab.d1, 9, 3);
                let v = tone + crack + speck(x, y, 29, 9) + fine / 4;
                ([28, 32, 44], [v, v, v + 2], 255)
            }
            Self::Sand => {
                // Fine grains: per-pixel grit with dark and pale grains mixed in.
                let grit = if noise(x ^ 0x5a4d, y).is_multiple_of(9) {
                    -24
                } else if noise(x, y ^ 0x5a4e).is_multiple_of(13) {
                    16
                } else {
                    0
                };
                let drift = (value_noise(x as f32 / 9.0, y as f32 / 5.0, 45) * 10.0) as i16 - 5;
                let v = fine + grit + drift;
                ([184, 156, 108], [v, v, v * 3 / 4], 255)
            }
            Self::Rubble => {
                // Broken rock: small angular chunks with dark gaps between.
                let cell = worley(x, y, 3.2, 2.6, 5);
                let tone = (cell.id % 17) as i16 - 8;
                let gap = crack_shade(cell.d2 - cell.d1, 20, 5);
                let lit = ((cell.py - y as f32) * 2.0 - (x as f32 - cell.px) * 1.5).clamp(-5.0, 6.0)
                    as i16;
                let v = tone + gap + lit + fine / 3;
                ([82, 88, 100], [v, v, v + 2], 255)
            }
            Self::Wood => {
                // Horizontal boards: streaky grain, dark seams, the odd knot.
                let warp = value_noise(x as f32 / 6.0, y as f32 / 2.0, 17) * 3.0;
                let grain = ((y as f32 * 2.1 + warp).sin() * 7.0) as i16;
                let seam = if (x + (y.div_euclid(3) * 5)).rem_euclid(13) == 0 {
                    -26
                } else {
                    0
                };
                let knot = if noise(x.div_euclid(3), y.div_euclid(2) ^ 0x7f).is_multiple_of(61) {
                    -18
                } else {
                    0
                };
                let v = grain + seam + knot + fine / 3;
                ([122, 72, 40], [v, v * 3 / 4, v / 2], 255)
            }
            Self::Coal => {
                // Near-black with a faint purple cast; faceted chunks and the
                // odd glassy glint that catches the light.
                let facet = worley(x, y, 5.0, 4.0, 29);
                let tone = (facet.id % 7) as i16 - 3;
                let edge = if facet.d2 - facet.d1 < 0.7 { -5 } else { 0 };
                if noise(x ^ 0x0c0a, y).is_multiple_of(17) {
                    ([104, 98, 132], [fine / 2; 3], 255)
                } else {
                    let v = tone + edge + fine / 4;
                    ([38, 34, 47], [v, v, v + 3], 255)
                }
            }
            Self::Ice => {
                let streak = ((x + y * 2).rem_euclid(11) == 0) as i16 * 26
                    + ((x * 2 - y).rem_euclid(17) == 0) as i16 * 12;
                let frost = (value_noise(x as f32 / 5.0, y as f32 / 5.0, 41) * 16.0) as i16 - 8;
                let v = fine / 2 + streak + frost;
                ([126, 200, 214], [v, v, v / 2], 235)
            }
            // Liquids and gases carry this per-pixel shade as they move.
            // Pigment belongs to the cell and travels with it. Low contrast grit
            // reads in motion without turning still pools into sparkling noise.
            Self::Water => ([49, 115, 109], [fine / 2, fine, fine * 3 / 4], 130),
            Self::Oil => {
                // Dark and viscous, with the odd iridescent fleck (violet,
                // green, gold) that travels with the oil as it moves.
                let v = fine / 3;
                match noise(x ^ 0x0e1, y) % 131 {
                    0 => ([84, 58, 96], [v; 3], 238),
                    1 => ([58, 86, 66], [v; 3], 238),
                    2 => ([112, 90, 42], [v; 3], 238),
                    _ => ([66, 54, 34], [v, v * 4 / 5, v / 2], 238),
                }
            }
            // Acid: a bright, glassy green.
            Self::Acid => ([96, 204, 50], [fine / 3, fine / 2, fine / 4], 150),
            // Blood: a deep, nearly opaque red with darker clots.
            Self::Blood => {
                let clot = if noise(x ^ 0xb100, y).is_multiple_of(23) {
                    -18
                } else {
                    0
                };
                ([122, 14, 20], [fine / 2 + clot, fine / 4, fine / 4], 220)
            }
            Self::Lava => {
                // Drops stay close to the pool's orange. The broad thermal
                // shapes are painted from the live shoreline instead.
                let roll = value_noise(x as f32 / 12.0, y as f32 / 9.0, 211);
                let v = (roll * 10.0) as i16 - 5;
                ([255, 155, 12], [0, v, v / 3], 255)
            }
            Self::VolcanicRock => {
                let crust = worley(x, y, 5.0, 3.5, 219);
                let seam = crack_shade(crust.d2 - crust.d1, 17, 4);
                let ember = if noise(x ^ 0x1a7a, y).is_multiple_of(37) {
                    18
                } else {
                    0
                };
                let v = fine / 3 + seam;
                ([55, 49, 51], [v + ember, v + ember / 3, v], 255)
            }
            Self::Fire => ([255, 142, 32], [fine; 3], 245),
            Self::Smoke => ([66, 62, 70], [coarse / 2 + fine / 3; 3], 130),
            Self::Steam => ([188, 210, 218], [coarse / 3 + fine / 4; 3], 130),
            Self::Fumes => ([172, 186, 128], [coarse / 2 + fine / 3; 3], 120),
            Self::Ash => {
                let v = fine + (value_noise(x as f32 / 3.0, y as f32 / 3.0, 53) * 12.0) as i16 - 6;
                ([104, 100, 99], [v; 3], 255)
            }
            Self::Masonry => {
                // Dressed blocks in running bond: a tone per block, dark
                // mortar joints, a lit upper edge and the odd chipped face.
                let course = y.div_euclid(5);
                let bx = x + course.rem_euclid(2) * 5;
                let block = noise(bx.div_euclid(10) ^ 0x3a5, course);
                let (u, v) = (bx.rem_euclid(10), y.rem_euclid(5));
                let tone = (block % 11) as i16 - 5;
                let joint = if u == 0 || v == 0 {
                    -22
                } else if v == 4 {
                    7
                } else if u == 1 {
                    4
                } else {
                    0
                };
                let chip = if noise(x ^ 0x6d, y).is_multiple_of(29) {
                    -9
                } else {
                    0
                };
                let v = tone + joint + chip + fine / 3;
                ([66, 56, 60], [v, v, v + 1], 255)
            }
            Self::Snow => {
                // Powder under the night sky, Noita's way: fine grains of
                // white and lilac with pale ice-blue ones among them, soft
                // drifts of cooler shade, faint ripples the wind combed into
                // it, and here and there a crystal that catches the light.
                let drift = (value_noise(x as f32 / 13.0, y as f32 / 6.0, 61) * 14.0) as i16 - 7;
                // Lumps: soft clods a few cells across, packed and shadowed
                // between, so a bank reads as a heap of snow, not paint.
                let clod = worley(x, y, 4.5, 3.5, 67);
                let lump = ((clod.py - y as f32) * 3.0 - (x as f32 - clod.px) * 1.2)
                    .clamp(-8.0, 9.0) as i16
                    - if clod.d2 - clod.d1 < 0.6 { 10 } else { 0 };
                let comb = value_noise(x as f32 / 9.0, y as f32 / 9.0, 62) * 6.0;
                let ripple = ((x as f32 * 0.19 + y as f32 * 0.85 + comb).sin() * 3.0) as i16;
                let grain = fine;
                let (tint, glint) = match noise(x ^ 0x5c0, y) % 47 {
                    0..=5 => ([-18, -10, 6], 0),
                    6..=8 => ([-8, -8, -4], -8),
                    9 | 10 => ([0, 0, 0], 30),
                    _ => ([0, 0, 0], 0),
                };
                let v = drift + ripple + grain + glint + lump;
                (
                    [210, 210, 232],
                    [v + tint[0], v + tint[1], v / 2 + tint[2] + 1],
                    255,
                )
            }
            Self::Metal => {
                // Dark iron plate: brushed streaks along it, a cool sheen,
                // the odd rivet catching the light, pitting here and there.
                let streak = (value_noise(x as f32 / 9.0, y as f32 * 1.4, 301) * 10.0) as i16 - 5;
                let rivet = if x.rem_euclid(7) == 3 && y.rem_euclid(6) == 2 {
                    26
                } else {
                    0
                };
                let pit = if noise(x ^ 0x3e7a, y).is_multiple_of(23) {
                    -12
                } else {
                    0
                };
                let v = streak + rivet + pit + fine / 4;
                ([72, 74, 84], [v, v, v + 3], 255)
            }
            Self::Rust => {
                // Flaking orange-brown, darker where it is wet with age.
                let mottle = (value_noise(x as f32 / 3.0, y as f32 / 3.0, 307) * 22.0) as i16 - 11;
                let v = fine + mottle;
                ([142, 74, 38], [v, v * 3 / 5, v / 3], 255)
            }
            Self::Glass => {
                // Clear and cold, faceted: pale bands where the facets catch
                // the light, a glint now and then.
                let facet = worley(x, y, 4.0, 4.0, 311);
                let band = if facet.d2 - facet.d1 < 0.7 { 24 } else { 0 };
                let glint = if noise(x ^ 0x6a55, y).is_multiple_of(19) {
                    40
                } else {
                    0
                };
                let v = (facet.id % 9) as i16 - 4 + band + glint;
                ([132, 196, 214], [v, v, v / 2], 190)
            }
            Self::Bone => {
                // Old bone: ivory, porous, stained at the ends.
                let pore = if noise(x ^ 0xb0e, y).is_multiple_of(7) {
                    -18
                } else {
                    0
                };
                let stain = (value_noise(x as f32 / 5.0, y as f32 / 5.0, 313) * 16.0) as i16 - 8;
                let v = fine / 2 + pore + stain;
                ([206, 194, 166], [v, v, v * 4 / 5], 255)
            }
            Self::Fungus => {
                // A soft, damp cap: violet-grey flesh speckled with pale
                // spots, a faint sheen of wet.
                let spot = if noise(x.div_euclid(2) ^ 0xf00, y.div_euclid(2)).is_multiple_of(9) {
                    44
                } else {
                    0
                };
                let mottle = (value_noise(x as f32 / 4.0, y as f32 / 4.0, 317) * 18.0) as i16 - 9;
                let v = fine / 2 + mottle;
                ([138, 96, 132], [v + spot, v + spot, v + spot * 3 / 4], 255)
            }
            Self::Web => {
                // Pale silk, thin enough to see through.
                let v = fine;
                ([206, 206, 214], [v; 3], 150)
            }
            Self::Flesh => {
                // Raw red meat marbled with darker veins and pale fat.
                let vein = ((value_noise(x as f32 / 4.0, y as f32 / 3.0, 83) - 0.5) * 30.0) as i16;
                let fat = if noise(x ^ 0xf1e5, y).is_multiple_of(11) {
                    34
                } else {
                    0
                };
                let v = fine / 2 + vein;
                (
                    [132, 50, 50],
                    [v + fat, v / 2 + fat, v / 2 + fat * 3 / 4],
                    255,
                )
            }
        };
        [
            (base[0] + variation[0]).clamp(0, 255) as u8,
            (base[1] + variation[1]).clamp(0, 255) as u8,
            (base[2] + variation[2]).clamp(0, 255) as u8,
            alpha,
        ]
    }

    pub fn optics(self) -> Optics {
        match self {
            Self::Lava => Optics {
                alpha: (248.0, 255.0),
                depth_tint: [1.0, 1.08, 0.72],
                churn: [0.0, 9.0, 2.0],
                drop: (18.0, 255),
                body_smooth: 0.8,
                surface_glint: [255.0, 228.0, 66.0],
                glint_strength: 0.42,
                reflect: 0.12,
                refract: 0.0,
                glow: [255.0, 124.0, 22.0],
                glow_light: 0.85,
                foam: 0.0,
            },
            // Translucent teal that keeps the back wall visible, glassy,
            // wobbling the view through it.
            Self::Water => Optics {
                alpha: (108.0, 163.0),
                depth_tint: [0.76, 0.82, 0.88],
                churn: [2.5; 3],
                drop: (5.0, 145),
                body_smooth: 0.78,
                surface_glint: [178.0, 230.0, 222.0],
                glint_strength: 0.22,
                reflect: 1.0,
                refract: 1.0,
                glow: [0.0; 3],
                glow_light: 0.0,
                foam: 0.45,
            },
            // Dark and nearly opaque, but its slick surface mirrors well.
            Self::Oil => Optics {
                alpha: (238.0, 238.0),
                depth_tint: [0.65, 0.65, 0.65],
                churn: [3.0, 3.0, 0.0],
                drop: (8.0, 245),
                body_smooth: 0.64,
                surface_glint: [188.0, 154.0, 104.0],
                glint_strength: 0.16,
                reflect: 0.8,
                refract: 0.15,
                glow: [0.0; 3],
                glow_light: 0.0,
                foam: 0.08,
            },
            // Glassy green deepening to emerald, with a faint toxic glow
            // that lights its surroundings a little.
            Self::Acid => Optics {
                alpha: (130.0, 190.0),
                depth_tint: [0.7, 0.84, 0.88],
                churn: [2.0; 3],
                drop: (14.0, 185),
                body_smooth: 0.74,
                surface_glint: [217.0, 255.0, 134.0],
                glint_strength: 0.25,
                reflect: 0.75,
                refract: 0.85,
                glow: [22.0, 52.0, 10.0],
                glow_light: 0.12,
                foam: 0.4,
            },
            // Nearly opaque, going almost black in depth; a dull surface.
            Self::Blood => Optics {
                alpha: (214.0, 252.0),
                depth_tint: [0.55, 0.55, 0.55],
                churn: [4.0, 1.2, 1.2],
                drop: (10.0, 230),
                body_smooth: 0.52,
                surface_glint: [226.0, 94.0, 84.0],
                glint_strength: 0.13,
                reflect: 0.45,
                refract: 0.3,
                glow: [0.0; 3],
                glow_light: 0.0,
                foam: 0.12,
            },
            _ => Optics {
                alpha: (245.0, 245.0),
                depth_tint: [1.0; 3],
                churn: [0.0; 3],
                drop: (8.0, 245),
                body_smooth: 0.0,
                surface_glint: [0.0; 3],
                glint_strength: 0.0,
                reflect: 0.0,
                refract: 0.0,
                glow: [0.0; 3],
                glow_light: 0.0,
                foam: 0.0,
            },
        }
    }

    /// Noita-style crust along exposed rock: lighter, rounded boulders stick
    /// out of the surface, some shallow, some reaching several cells in,
    /// each lit from above-left and broken into pebbles with dark gaps, so a
    /// cave wall reads as weathered rather than as a flat cut. `depth` is
    /// the distance to the nearest open cell (1 on the surface).
    pub fn edge_crust(self, x: i32, y: i32, depth: u8) -> Option<[u8; 3]> {
        let (base, reach): ([i16; 3], u8) = match self {
            Self::Stone => ([108, 113, 121], 9),
            Self::VolcanicRock => ([94, 80, 77], 3),
            Self::Bedrock => ([52, 58, 74], 3),
            Self::Coal => ([62, 56, 74], 3),
            _ => return None,
        };
        if depth == 0 || depth > reach {
            return None;
        }
        let boulder = worley(x, y, 10.5, 7.5, 131);
        let size = (2 + boulder.id % 6 + (boulder.id >> 5) % 3) as u8;
        let round = boulder.d1 < 3.8 + ((boulder.id >> 8) % 4) as f32 * 0.8;
        let in_boulder = !boulder.id.is_multiple_of(5) && round && depth <= size.min(reach);
        // Between boulders, loose pebbles still line the very surface.
        let pebbly = depth == 1 && value_noise(x as f32 / 2.2, y as f32 / 2.0, 137) > 0.5;
        if !in_boulder && !pebbly {
            return None;
        }
        let pebble = worley(x, y, 3.0, 2.5, 139);
        let tone = (pebble.id % 17) as i16 - 8;
        let gap = pebble.d2 - pebble.d1 < 0.6;
        let (cx, cy) = if in_boulder {
            (boulder.px, boulder.py)
        } else {
            (pebble.px, pebble.py)
        };
        let lit = ((cy - y as f32) * 2.2 - (x as f32 - cx) * 1.4).clamp(-10.0, 13.0) as i16;
        let fade = (depth as i16 - 1) * 5;
        let v = if gap { -30 } else { tone + lit - fade };
        Some(base.map(|c| (c + v).clamp(0, 255) as u8))
    }
}
