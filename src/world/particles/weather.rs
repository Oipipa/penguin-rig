//! The First Stop's weather, and snow underfoot.
//!
//! One wind blows along the shore, gusting from the west (the title's wind:
//! a slow swell with gusts riding on it, each gust crossing the shore from
//! west to east). Snow falls in it, flake by flake, at a slant that
//! steepens and flattens as the gusts come and go, and melts where it
//! lands. In the stronger gusts the wind tears real snow off the crests of
//! the drifts and carries it east to drop it again. Frost glints on the
//! snow here and there, catching what light there is. Out of the hole at
//! the end of the shore warm air breathes up, a wisp at a time.
//!
//! Underfoot the snow is real too: the penguin's feet kick it up as they
//! go (a little of it flies, a dust of it hangs a moment), and come down
//! from a jump into a puff of it. And in the cold its breath clouds.

use super::*;

/// Flakes falling at once, over the view.
const FLAKES: usize = 1_800;
/// Cells of snow a gust may tear off the drifts in a tick, at most.
const SPINDRIFT: usize = 3;

impl LevelGrid {
    /// A little snow can blow into the mouth, but the long shaft is still.
    fn snowfall_floor(&self) -> i32 {
        0
    }

    /// Fill the opening view with flakes before the first simulation tick,
    /// so the First Stop is already snowing when play begins.
    pub fn seed_snowfall(&mut self, around: Vec2) {
        if !self.has_weather() {
            return;
        }
        self.motes
            .retain(|mote| !matches!(mote.kind, ParticleKind::Snowflake));
        let center = self.world_to_cell(around);
        let x0 = (center.x - 260).max(0);
        let x1 = (center.x + 260).min(self.width - 1);
        let y0 = (center.y - 150).max(self.snowfall_floor());
        let y1 = (center.y + 170).min(self.height - 1);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        self.snow_view = Some(render::CellRect { x0, y0, x1, y1 });
        for k in 0..FLAKES {
            let dice = crate::world::materials::noise(k as i32 * 7919 + 83, 29);
            let cell = GridPosition {
                x: x0 + (dice % (x1 - x0 + 1) as u32) as i32,
                y: y0 + ((dice >> 11) % (y1 - y0 + 1) as u32) as i32,
            };
            // (Packed away or not: never inside the rock.)
            if self.material_anywhere(cell).is_some() {
                continue;
            }
            let at = self.cell_to_world(cell);
            self.motes.push(Mote {
                position: at,
                previous: at,
                velocity: Vec2::new(30.0, -40.0),
                kind: ParticleKind::Snowflake,
                energy: 2400.0,
                phase: ((dice >> 21) % 1000) as f32 / 1000.0,
            });
        }
        self.dirty = true;
    }

    /// The camera can reveal air faster than flakes can blow into it. Fill
    /// only the newly seen outdoor part of its (offscreen-padded) view
    /// before rendering the frame in which it appears.
    pub(in crate::world) fn prime_snowfall_view(&mut self) {
        if !self.has_weather() || self.cells.is_empty() {
            return;
        }
        let Some(view) = self.view else {
            return;
        };
        let view = view.intersect(render::CellRect {
            x0: 0,
            y0: self.snowfall_floor(),
            x1: self.width - 1,
            y1: self.height - 1,
        });
        if view.is_empty() || self.snow_view == Some(view) {
            return;
        }
        let overlap = self
            .snow_view
            .map_or(render::CellRect::EMPTY, |old| view.intersect(old));
        if overlap.is_empty() {
            self.fill_snowfall_area(view);
        } else {
            for area in [
                render::CellRect {
                    x0: view.x0,
                    x1: overlap.x0 - 1,
                    y0: view.y0,
                    y1: view.y1,
                },
                render::CellRect {
                    x0: overlap.x1 + 1,
                    x1: view.x1,
                    y0: view.y0,
                    y1: view.y1,
                },
                render::CellRect {
                    x0: overlap.x0,
                    x1: overlap.x1,
                    y0: view.y0,
                    y1: overlap.y0 - 1,
                },
                render::CellRect {
                    x0: overlap.x0,
                    x1: overlap.x1,
                    y0: overlap.y1 + 1,
                    y1: view.y1,
                },
            ] {
                if !area.is_empty() {
                    self.fill_snowfall_area(area);
                }
            }
        }
        self.snow_view = Some(view);
    }

    fn fill_snowfall_area(&mut self, area: render::CellRect) {
        if self.motes.len() >= MAX_MOTES - 256 {
            return;
        }
        // Match the opening snowfall's density, with stochastic rounding so
        // a one-cell camera move does not systematically overfill each strip.
        let area_cells = (area.width() * area.height()) as usize;
        let seed_cells = 521 * 321;
        let expected = area_cells * FLAKES;
        let round = crate::world::materials::noise(area.x0 ^ self.tick as i32, area.y0 ^ area.x1)
            as usize
            % seed_cells;
        let desired = expected / seed_cells + usize::from(round < expected % seed_cells);
        let present = self
            .motes
            .iter()
            .filter(|mote| {
                matches!(mote.kind, ParticleKind::Snowflake)
                    && area.contains(self.world_to_cell(mote.position))
            })
            .count();
        let needed = desired.saturating_sub(present);
        let mut added = 0;
        for k in 0..needed * 3 {
            if added >= needed || self.motes.len() >= MAX_MOTES - 256 {
                break;
            }
            let dice = crate::world::materials::noise(
                (k as i32).wrapping_mul(7919) ^ area.x0.wrapping_mul(71) ^ self.tick as i32,
                area.y0.wrapping_mul(83) ^ area.x1.wrapping_mul(131),
            );
            let cell = GridPosition {
                x: area.x0 + (dice % area.width() as u32) as i32,
                y: area.y0 + ((dice >> 11) % area.height() as u32) as i32,
            };
            // (Packed away or not: never inside the rock.)
            if self.material_anywhere(cell).is_some() {
                continue;
            }
            let at = self.cell_to_world(cell);
            self.motes.push(Mote {
                position: at,
                previous: at,
                velocity: Vec2::new(30.0, -40.0),
                kind: ParticleKind::Snowflake,
                energy: 2400.0,
                phase: ((dice >> 21) % 1000) as f32 / 1000.0,
            });
            added += 1;
        }
        self.dirty |= added > 0;
    }

    /// Whether this level has weather (snow, wind).
    fn has_weather(&self) -> bool {
        self.weather
    }

    /// How hard the wind blows at column `x` now: about 1 in a breeze,
    /// swelling to 1.5 or so in a gust, the gust reaching the east end of
    /// the shore after the west.
    pub fn wind_at(&self, x: i32) -> f32 {
        let t = self.tick as f32 / 60.0 + 918.0 - x as f32 / 260.0;
        let swell = smooth_noise(t * 0.21, 11) * 0.5 + smooth_noise(t * 0.47, 23) * 0.3;
        let gust = smooth(-0.05, 0.5, swell);
        let flurry = smooth_noise(t * 1.7, 37) * 0.12 * (0.4 + gust);
        1.0 + gust * 0.55 + swell * 0.12 + flurry
    }

    /// The weather, a tick of it, round the view.
    pub(in crate::world) fn step_weather(&mut self, player: Option<Vec2>) {
        if !self.has_weather() || self.cells.is_empty() {
            return;
        }
        let Some(around) = self.view.or_else(|| {
            player.map(|p| {
                let c = self.world_to_cell(p);
                render::CellRect {
                    x0: c.x - 260,
                    y0: c.y - 150,
                    x1: c.x + 260,
                    y1: c.y + 170,
                }
            })
        }) else {
            return;
        };
        let (x0, x1) = (around.x0.max(0), around.x1.min(self.width - 1));
        let (y0, y1) = (
            around.y0.max(self.snowfall_floor()),
            around.y1.min(self.height - 1),
        );
        if y1 < self.snowfall_floor() {
            self.motes
                .retain(|mote| !matches!(mote.kind, ParticleKind::Snowflake));
            return;
        }
        if x1 <= x0 || y1 <= y0 || self.motes.len() >= MAX_MOTES - 256 {
            return;
        }
        // Snow that has blown well past the camera must not use the whole
        // weather budget while the penguin walks to another part of the
        // shore. Keep a margin so entering flakes never pop at the edge.
        let origin = Vec2::new(self.width as f32 * 0.5, self.height as f32 * 0.5);
        self.motes.retain(|mote| {
            if !matches!(mote.kind, ParticleKind::Snowflake) {
                return true;
            }
            let p = mote.position / self.pixel_size + origin;
            p.x >= (x0 - 80) as f32
                && p.x <= (x1 + 80) as f32
                && p.y >= (y0 - 45) as f32
                && p.y <= (y1 + 45) as f32
        });
        let flakes = self
            .motes
            .iter()
            .filter(|m| matches!(m.kind, ParticleKind::Snowflake))
            .count();
        let tick = self.tick as i32;
        let roll = |salt: i32| crate::world::materials::noise(tick.wrapping_mul(7919) ^ salt, salt);
        // Snow falls: new flakes come in along the top of the view (and in
        // from its windward side), more of them while there are few.
        let fresh = if flakes < FLAKES / 2 {
            20
        } else if flakes < FLAKES {
            10
        } else {
            0
        };
        for k in 0..fresh {
            let dice = roll(k * 131 + 7);
            let along = (dice % 1000) as f32 / 1000.0;
            let cell = if k % 3 == 2 {
                GridPosition {
                    x: x0 + (dice >> 10) as i32 % 6,
                    y: y0 + (along * (y1 - y0) as f32) as i32,
                }
            } else {
                GridPosition {
                    x: x0 - 40 + (along * (x1 - x0 + 80) as f32) as i32,
                    y: y1 - (dice >> 12) as i32 % 8,
                }
            };
            if self.index(cell).is_none() || self.get(cell).is_some() {
                continue;
            }
            let phase = ((dice >> 16) % 1000) as f32 / 1000.0;
            let at = self.cell_to_world(cell);
            self.motes.push(Mote {
                position: at,
                previous: at,
                velocity: Vec2::new(30.0, -40.0),
                kind: ParticleKind::Snowflake,
                energy: 2400.0,
                phase,
            });
        }
        // In a gust, snow streams off the crests of the drifts.
        let mut torn = 0;
        for k in 0..24 {
            if torn >= SPINDRIFT {
                break;
            }
            let dice = roll(k * 211 + 3);
            let x = x0 + (dice % (x1 - x0).max(1) as u32) as i32;
            let wind = self.wind_at(x);
            if wind < 1.18 {
                continue;
            }
            let Some(top) = self.snow_top(x, y0, y1) else {
                continue;
            };
            // A crest: the snow upwind of it lies lower.
            let upwind = self.snow_top(x - 2, y0, y1).unwrap_or(top);
            let chance = (wind - 1.18) * 2.2 + if upwind < top { 0.25 } else { 0.0 };
            if ((dice >> 12) % 1000) as f32 / 1000.0 > chance {
                continue;
            }
            let cell = GridPosition { x, y: top };
            let lift = ((dice >> 20) % 100) as f32;
            if self.launch_snow(cell, Vec2::new(90.0 * wind + lift, 30.0 + lift * 0.6)) {
                torn += 1;
                let at = self.cell_to_world(cell);
                self.motes.push(Mote {
                    position: at,
                    previous: at,
                    velocity: Vec2::new(70.0 * wind, 25.0),
                    kind: ParticleKind::Dust(TerrainKind::Snow),
                    energy: 150.0,
                    phase: 0.0,
                });
            }
        }
        // Frost glints, here and there on the snow.
        for k in 0..3 {
            let dice = roll(k * 97 + 41);
            let x = x0 + (dice % (x1 - x0).max(1) as u32) as i32;
            if let Some(top) = self.snow_top(x, y0, y1) {
                let at = self.cell_to_world(GridPosition { x, y: top });
                self.spark(
                    at + Vec2::Y * 0.5,
                    Vec2::ZERO,
                    Glow::Glint,
                    14.0 + (dice >> 20) as f32 % 18.0,
                );
            }
        }
    }

    /// The top cell of the snow lying on column `x` between rows `y0` and
    /// `y1`, if snow is what lies on top there.
    fn snow_top(&self, x: i32, y0: i32, y1: i32) -> Option<i32> {
        let top = (y0..=y1)
            .rev()
            .find(|&y| self.get(GridPosition { x, y }).is_some())?;
        (self.get(GridPosition { x, y: top }) == Some(TerrainKind::Snow)
            && self.get(GridPosition { x, y: top + 1 }).is_none())
        .then_some(top)
    }

    /// Throw the snow at `cell` into the air with `velocity`: whether it
    /// went (it must be snow, with open air over it).
    fn launch_snow(&mut self, cell: GridPosition, velocity: Vec2) -> bool {
        let Some(i) = self.index(cell) else {
            return false;
        };
        let above = GridPosition {
            x: cell.x,
            y: cell.y + 1,
        };
        if self.cells[i] != Some(TerrainKind::Snow) || self.get(above).is_some() {
            return false;
        }
        let grain = Grain {
            position: self.cell_to_world(cell) + Vec2::Y * self.pixel_size * 0.9,
            velocity,
            kind: TerrainKind::Snow,
            temperature: self.temperature[i],
            fuel: self.life[i],
            color: self.base_colors[i],
            ghost: 0,
            rest: 0,
        };
        self.put(cell, None);
        self.reserve_grain(grain.position, Some(TerrainKind::Snow));
        self.grains.push(grain);
        true
    }

    /// The penguin's feet in snow: `feet` where they are, `velocity` how it
    /// moves, `landing` how hard it has just come down (0 if it has not).
    /// Snow under and before the feet is kicked up (back from the way it
    /// goes, or out both ways on a landing), and a dust of it hangs.
    pub fn kick_snow(&mut self, feet: Vec2, velocity: Vec2, landing: f32) {
        let cell = self.world_to_cell(feet);
        let tick = self.tick as i32;
        let dice = crate::world::materials::noise(cell.x ^ tick.wrapping_mul(31), cell.y ^ 0x5a0);
        let speed = velocity.x.abs();
        let kicks = if landing > 0.0 {
            2 + (landing / 140.0) as usize
        } else if speed > 40.0 && dice % 100 < (speed * 0.12) as u32 {
            1
        } else {
            0
        };
        if kicks == 0 {
            return;
        }
        let back = -velocity.x.signum();
        let mut kicked = 0;
        for k in 0..kicks.min(8) * 3 {
            if kicked >= kicks.min(8) {
                break;
            }
            let roll = crate::world::materials::noise(dice as i32 ^ k as i32, 0x5a1);
            let side = if landing > 0.0 {
                if k % 2 == 0 { 1 } else { -1 }
            } else {
                back as i32
            };
            let x = cell.x + side * (1 + (roll % 4) as i32);
            let Some(top) = (cell.y - 4..=cell.y + 2)
                .rev()
                .find(|&y| self.get(GridPosition { x, y }) == Some(TerrainKind::Snow))
            else {
                continue;
            };
            let out = 40.0 + (roll >> 8) as f32 % 70.0 + speed * 0.25 + landing * 0.2;
            let up = 70.0 + (roll >> 16) as f32 % 90.0 + landing * 0.35;
            if self.launch_snow(GridPosition { x, y: top }, Vec2::new(side as f32 * out, up)) {
                kicked += 1;
            }
        }
        // A dust of it, hanging a moment.
        let dust = 2 + kicks * 2;
        for k in 0..dust {
            let roll = crate::world::materials::noise(dice as i32 ^ (k as i32 * 977), 0x5a2);
            let spread = (roll % 100) as f32 / 100.0 - 0.5;
            let side = if landing > 0.0 {
                spread * 2.0
            } else {
                back + spread * 0.6
            };
            self.motes.push(Mote {
                position: feet + Vec2::new(spread * 6.0, 1.0),
                previous: feet,
                velocity: Vec2::new(
                    side * (40.0 + speed * 0.2),
                    30.0 + (roll >> 8) as f32 % 50.0,
                ),
                kind: ParticleKind::Dust(TerrainKind::Snow),
                energy: 120.0 + (roll >> 16) as f32 % 80.0,
                phase: 0.0,
            });
        }
    }

    /// Something heavy coming down in snow at `feet` (a penguin falling a
    /// long way onto its face): the snow `half` cells either side is thrown
    /// up and out from where it hit, more the nearer, `strength` (0..1) of
    /// a full splash, and a cloud of it hangs.
    pub fn splash_snow(&mut self, feet: Vec2, half: i32, strength: f32) {
        let middle = self.world_to_cell(feet);
        let tick = self.tick as i32;
        for dx in -half..=half {
            let x = middle.x + dx;
            let near = 1.0 - dx.abs() as f32 / (half as f32 + 1.0);
            // Two layers off the top, where it is near.
            for layer in 0..1 + (near > 0.5) as i32 {
                let Some(top) = self.snow_top(x, middle.y - 8, middle.y + 4) else {
                    break;
                };
                let roll = crate::world::materials::noise(x ^ tick.wrapping_mul(17), layer ^ 0x5b3);
                let jitter = (roll % 100) as f32 / 100.0;
                let side = if dx == 0 {
                    if roll.is_multiple_of(2) { 1.0 } else { -1.0 }
                } else {
                    dx.signum() as f32
                };
                let velocity = Vec2::new(
                    side * (50.0 + 90.0 * (1.0 - near) + 50.0 * jitter),
                    (120.0 + 200.0 * near + 60.0 * jitter) * (1.0 - 0.3 * layer as f32),
                ) * strength;
                self.launch_snow(GridPosition { x, y: top }, velocity);
            }
        }
        for k in 0..18 {
            let roll = crate::world::materials::noise(middle.x ^ (k * 977), 0x5b4 ^ tick);
            let spread = (roll % 100) as f32 / 100.0 - 0.5;
            self.motes.push(Mote {
                position: feet + Vec2::new(spread * 40.0, 2.0),
                previous: feet,
                velocity: Vec2::new(spread * 160.0, 40.0 + (roll >> 8) as f32 % 90.0) * strength,
                kind: ParticleKind::Dust(TerrainKind::Snow),
                energy: 140.0 + (roll >> 16) as f32 % 90.0,
                phase: 0.0,
            });
        }
    }

    /// Take up to `most` cells of snow off the top of the snow between
    /// `from` and `to` (world x, along the snow's surface near height `y`):
    /// a dent pressed into it. The snow taken, to be put back somewhere
    /// (see [`Lump`]): nothing is made or lost.
    pub fn scoop_snow(&mut self, from: f32, to: f32, y: f32, most: usize) -> Vec<Lump> {
        let (a, b) = (
            self.world_to_cell(Vec2::new(from.min(to), y)),
            self.world_to_cell(Vec2::new(from.max(to), y)),
        );
        let mut taken = Vec::new();
        // A layer at a time, so the dent is even.
        for _ in 0..3 {
            for x in a.x..=b.x {
                if taken.len() >= most {
                    return taken;
                }
                if let Some(top) = self.snow_top(x, a.y - 6, a.y + 4) {
                    let at = GridPosition { x, y: top };
                    let i = self.index(at).unwrap();
                    taken.push(Lump {
                        temperature: self.temperature[i],
                        life: self.life[i],
                        color: self.base_colors[i],
                    });
                    self.put(at, None);
                }
            }
        }
        taken
    }

    /// A lump of snow let go of at `at`, falling as a grain with
    /// `velocity` (snow coming off a penguin as it gets up).
    pub fn shed_snow(&mut self, lump: Lump, at: Vec2, velocity: Vec2) {
        let grain = Grain {
            position: at,
            velocity,
            kind: TerrainKind::Snow,
            temperature: lump.temperature,
            fuel: lump.life,
            color: lump.color,
            ghost: 6,
            rest: 0,
        };
        self.reserve_grain(grain.position, Some(TerrainKind::Snow));
        self.grains.push(grain);
    }

    /// Lumps of snow put down over `at` in a little mound resting on what
    /// is under it, only where there is open air (the snow a penguin got
    /// up out of a drift with, on its head): the middle first, then out to
    /// the sides, row on row. What found no room falls as grains instead.
    pub fn heap_snow(&mut self, at: Vec2, lumps: &[Lump]) {
        let mut middle = self.world_to_cell(at);
        if self.cell_to_world(middle).y - self.pixel_size * 0.5 < at.y - 0.01 {
            middle.y += 1;
        }
        let mut lumps = lumps.iter();
        'rows: for row in 0..5 {
            let half = (4 - row).max(1);
            for k in 0..=2 * half {
                let dx = if k % 2 == 0 { k / 2 } else { -(k + 1) / 2 };
                let cell = GridPosition {
                    x: middle.x + dx,
                    y: middle.y + row,
                };
                if self.get(cell).is_some() {
                    continue;
                }
                let Some(lump) = lumps.next() else {
                    break 'rows;
                };
                self.put(cell, Some(TerrainKind::Snow));
                let i = self.index(cell).unwrap();
                self.base_colors[i] = lump.color;
                self.temperature[i] = lump.temperature;
                self.life[i] = lump.life;
            }
        }
        for (k, &lump) in lumps.enumerate() {
            let spot = self.cell_to_world(middle) + Vec2::new((k % 7) as f32 * 2.0 - 6.0, 8.0);
            self.shed_snow(lump, spot, Vec2::ZERO);
        }
    }

    /// A breath clouding in the cold at `mouth`, blown toward `facing` and
    /// off on the wind.
    pub fn breathe(&mut self, mouth: Vec2, facing: f32) {
        if !self.has_weather() {
            return;
        }
        let cell = self.world_to_cell(mouth);
        for k in 0..5 {
            let roll = crate::world::materials::noise(cell.x ^ (self.tick as i32 * 13) ^ k, 0xb4e);
            let spread = (roll % 100) as f32 / 100.0 - 0.5;
            self.motes.push(Mote {
                position: mouth,
                previous: mouth,
                velocity: Vec2::new(facing * (26.0 + k as f32 * 6.0), 8.0 + spread * 14.0),
                kind: ParticleKind::Vapor,
                energy: 70.0 + k as f32 * 12.0,
                phase: 70.0 + k as f32 * 12.0,
            });
        }
    }

    /// A flake falls in the wind, swaying, and melts where it lands.
    pub(super) fn step_snowflake(&mut self, mote: &mut Mote, dt: f32) -> bool {
        mote.energy -= dt * 60.0;
        if mote.energy <= 0.0 || self.world_to_cell(mote.position).y < self.snowfall_floor() {
            return false;
        }
        let cell = self.world_to_cell(mote.position);
        let wind = self.wind_at(cell.x);
        let t = self.tick as f32 / 60.0;
        // The distant powder hangs in the air; the flakes nearest the
        // camera cross it faster, giving the storm its depth in motion.
        let depth = mote.phase;
        let fall = 24.0 + depth * 44.0;
        let sway = (t * (1.4 + depth * 1.7) + depth * 47.0).sin() * (7.0 + depth * 14.0);
        let eddy = smooth_noise(t * 0.8 - cell.x as f32 / 170.0, 61) * 12.0;
        let target = Vec2::new((wind - 0.58) * (55.0 + depth * 48.0) + sway + eddy, -fall);
        mote.velocity = mote.velocity.lerp(target, (dt * 2.5).min(1.0));
        let next = mote.position + mote.velocity * dt;
        // (A flake over a part of the world not yet brought in falls on
        // through it as air: it is only lost leaving the world. Losing it
        // there emptied the opening snowfall over all the unloaded sky.)
        let at = self.world_to_cell(next);
        let outside = at.x < 0 || at.x >= self.width || at.y >= self.height;
        if at.y < self.snowfall_floor()
            || outside
            || self.material_at(next).is_some_and(|kind| !kind.is_gas())
        {
            return false;
        }
        mote.position = next;
        true
    }

    /// A wisp of breath or warm air: it rises a little, drifts on the
    /// wind, spreads and thins out.
    pub(super) fn step_vapor(&mut self, mote: &mut Mote, dt: f32) -> bool {
        mote.energy -= dt * 60.0;
        if mote.energy <= 0.0 {
            return false;
        }
        let cell = self.world_to_cell(mote.position);
        let wind = if self.has_weather() {
            self.wind_at(cell.x)
        } else {
            0.0
        };
        // Out of the wind (down the hole) it only rises.
        let sheltered = self.render_cache.sky_share(&self.chunks, cell) < 0.4;
        let drift = if sheltered { 0.0 } else { (wind - 0.5) * 30.0 };
        mote.velocity = mote
            .velocity
            .lerp(Vec2::new(drift, 14.0), (dt * 1.6).min(1.0));
        let next = mote.position + mote.velocity * dt;
        if self.material_at(next).is_some_and(|kind| !kind.is_gas()) {
            mote.velocity *= 0.3;
        } else {
            mote.position = next;
        }
        true
    }
}

fn smooth(start: f32, end: f32, value: f32) -> f32 {
    let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smooth noise in [-1, 1] along one axis.
fn smooth_noise(t: f32, seed: u32) -> f32 {
    let i = t.floor();
    let f = t - i;
    let k = i as i32;
    let height = |k: i32| {
        (crate::world::materials::noise(k.wrapping_mul(0x9E37), seed as i32) % 10_000) as f32
            / 5000.0
            - 1.0
    };
    let (a, b) = (height(k), height(k + 1));
    let e = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    a + (b - a) * e
}
