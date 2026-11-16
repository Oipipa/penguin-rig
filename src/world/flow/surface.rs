//! What moves a liquid's surface: pixels breaking away as drops, splashing
//! back, diffusing into another liquid, wandering idly; drops thrown, cells
//! advected along their flow, and impulses from blasts and bodies.

use super::*;

impl LevelGrid {
    /// Falling liquid does not stay in one piece: a pixel with open air
    /// beside it drifts off to that side now and then, and one leading a
    /// fast-falling body sometimes tears off as a droplet of its own. The
    /// body frays at its edges and sprays from its front as it falls.
    /// Returns where the pixel went, if it left the grid or moved.
    pub(super) fn break_up(
        &mut self,
        cell: GridPosition,
        kind: TerrainKind,
    ) -> Option<GridPosition> {
        let i = self.index(cell)?;
        let below = GridPosition {
            x: cell.x,
            y: cell.y - 1,
        };
        let above = GridPosition {
            x: cell.x,
            y: cell.y + 1,
        };
        if !self.open_for_liquid(below) {
            return None;
        }
        let dice = self.roll(cell, 52);
        let speed = -self.flow[i].y;
        let viscous = (kind.liquid_style().spread / 2.0).min(1.0);
        if speed > 1.4
            && self.get(above).is_some_and(TerrainKind::is_liquid)
            && ((dice % 1000) as f32) < TEAR as f32 * viscous * (speed - 1.4).min(1.0)
        {
            // The torn-off drop keeps the fall, with its own small sideways
            // kick and a little air braking.
            let fall = speed * self.pixel_size * 60.0;
            let side = ((dice >> 10) % 200) as f32 / 100.0 - 1.0;
            let velocity = Vec2::new(
                self.flow[i].x * self.pixel_size * 60.0 + side * (25.0 + fall * 0.12),
                -fall * (0.72 + ((dice >> 18) % 20) as f32 * 0.01),
            );
            let grain = Grain {
                position: self.cell_to_world(cell),
                velocity,
                kind,
                temperature: self.temperature[i],
                fuel: self.life[i],
                color: self.base_colors[i],
                ghost: 0,
                rest: 0,
            };
            self.put(cell, None);
            self.reserve_grain(grain.position, Some(kind));
            self.grains.push(grain);
            return Some(cell);
        }
        if ((dice >> 12) % 1000) as f32 >= FRAY as f32 * viscous {
            return None;
        }
        let side = if (dice >> 22) & 1 == 0 { 1 } else { -1 };
        for side in [side, -side] {
            let beside = GridPosition {
                x: cell.x + side,
                y: cell.y,
            };
            let across = GridPosition {
                x: cell.x - side,
                y: cell.y,
            };
            let diagonal = GridPosition {
                x: beside.x,
                y: cell.y - 1,
            };
            // Only an edge frays: the pixel's other side is liquid, so it
            // is part of a falling body, not a lone drop drifting about.
            if self.open_for_liquid(beside)
                && self.open_for_liquid(diagonal)
                && self.get(across).is_some_and(TerrainKind::is_liquid)
            {
                self.swap_cells(cell, diagonal);
                let j = self.index(diagonal).unwrap();
                self.flow[j].x += side as f32 * 0.25;
                return Some(diagonal);
            }
        }
        None
    }

    /// A landing pixel sometimes bounces back up as a droplet, so a pour
    /// sprays where it hits instead of vanishing into the pool.
    pub(super) fn splash_back(&mut self, cell: GridPosition, impact: f32) -> bool {
        let chance = ((impact - SPLASH_IMPACT) * 0.1).min(0.2);
        let dice = self.roll(cell, 40);
        if (dice % 1000) as f32 >= chance * 1000.0
            || !self.open_for_liquid(GridPosition {
                x: cell.x,
                y: cell.y + 1,
            })
        {
            return false;
        }
        let side = if (dice >> 10) & 1 == 0 { 1.0 } else { -1.0 };
        let speed = impact * self.pixel_size * 60.0;
        let velocity = Vec2::new(
            side * speed * (0.08 + ((dice >> 11) % 100) as f32 * 0.0022),
            speed * (0.16 + ((dice >> 18) % 100) as f32 * 0.002),
        );
        self.launch_droplet(cell, velocity);
        true
    }

    /// Trade places with a different water-based neighbor, picked by dice.
    /// Returns where this pixel went.
    pub(super) fn diffuse(
        &mut self,
        cell: GridPosition,
        kind: TerrainKind,
    ) -> Option<GridPosition> {
        let (dx, dy) = [(1, 0), (-1, 0), (0, 1), (0, -1)][(self.roll(cell, 49) % 4) as usize];
        let to = GridPosition {
            x: cell.x + dx,
            y: cell.y + dy,
        };
        let j = self.index(to)?;
        let other = self.cells[j]?;
        if other == kind
            || !other.is_aqueous()
            || self.updated[j] == self.stamp()
            || self.grain_at(to)
            || self.body_at(to)
        {
            return None;
        }
        self.diffusing = true;
        self.swap_cells(cell, to);
        self.diffusing = false;
        Some(to)
    }

    /// An idle surface pixel with open, supported room beside it hops over.
    /// Returns where it went.
    pub(super) fn wander(&mut self, cell: GridPosition, kind: TerrainKind) -> Option<GridPosition> {
        let chance = match kind {
            TerrainKind::Oil => WANDER_OIL,
            TerrainKind::Blood => WANDER_BLOOD,
            _ => WANDER_WATER,
        };
        let room = |grid: &Self, side: i32| {
            let beside = GridPosition {
                x: cell.x + side,
                y: cell.y,
            };
            let under = GridPosition {
                x: beside.x,
                y: cell.y - 1,
            };
            (grid.open_for_liquid(beside) && !grid.open_for_liquid(under)).then_some(beside)
        };
        let dice = self.roll(cell, 44);
        if dice % 1000 >= chance {
            // Free to hop on a later tick: the surface stays restless.
            if room(self, 1).is_some() || room(self, -1).is_some() {
                self.keep_awake(cell);
            }
            return None;
        }
        let side = if (dice >> 10) & 1 == 0 { 1 } else { -1 };
        let Some(beside) = room(self, side) else {
            if room(self, -side).is_some() {
                self.keep_awake(cell);
            }
            return None;
        };
        // Hopping along its own row changes no head of pressure.
        self.quiet_moves = true;
        self.swap_cells(cell, beside);
        self.quiet_moves = false;
        Some(beside)
    }

    /// Throw the liquid pixel at `cell` into the air as a conserved droplet;
    /// it rejoins the grid where it lands. The cell above must be open.
    pub(in crate::world) fn launch_droplet(&mut self, cell: GridPosition, velocity: Vec2) {
        let Some(i) = self.index(cell) else { return };
        let Some(kind) = self.cells[i].filter(|kind| kind.is_liquid()) else {
            return;
        };
        let grain = Grain {
            // Start beyond the exposed face so the first swept step cannot
            // immediately stick to the neighboring surface cell.
            position: self.cell_to_world(cell) + Vec2::Y * self.pixel_size * 0.9,
            velocity,
            kind,
            temperature: self.temperature[i],
            fuel: self.life[i],
            color: self.base_colors[i],
            ghost: 0,
            rest: 0,
        };
        self.put(cell, None);
        self.reserve_grain(grain.position, Some(kind));
        self.grains.push(grain);
    }

    pub(super) fn advect_liquid(
        &mut self,
        cell: &mut GridPosition,
        kind: TerrainKind,
        player: Option<Vec2>,
        grounded: bool,
    ) {
        let mut i = self.index(*cell).unwrap();
        if self.flow[i].x.abs() < 0.08 {
            self.flow[i].x = 0.0;
            self.phase[i].x = 0.0;
            return;
        }
        let side = self.flow[i].x.signum() as i32;
        self.phase[i].x += self.flow[i].x.abs();
        for _ in 0..(self.phase[i].x as i32).min(MAX_CELL_SPEED as i32) {
            let next = GridPosition {
                x: cell.x + side,
                y: cell.y,
            };
            let below = GridPosition {
                x: cell.x,
                y: cell.y - 1,
            };
            let next_below = GridPosition {
                x: next.x,
                y: next.y - 1,
            };
            if grounded
                && self.get(below).is_some_and(TerrainKind::is_solid)
                && !self
                    .get(GridPosition {
                        x: cell.x,
                        y: cell.y + 1,
                    })
                    .is_some_and(TerrainKind::is_liquid)
                && self.flow[i].x.abs() < 1.2
                && self.open_for_liquid(next_below)
            {
                // Leave the last wet pixel attached to the rim. Liquid piled
                // above it provides the head needed to overflow.
                self.flow[i].x = 0.0;
                self.phase[i].x = 0.0;
                return;
            }
            // A cell vacated earlier this tick is only sometimes taken, so a pull
            // fades with distance from an opening instead of sliding the whole
            // row along like a conveyor in the update order. Faster, deeper flow
            // (higher pressure) drags liquid from further back.
            let fresh_take = (0.25 + self.flow[i].x.abs() * 0.18).min(0.92);
            if grounded
                && self
                    .index(next)
                    .is_some_and(|j| self.updated[j] == self.stamp())
                && (self.roll(next, 3) % 1000) as f32 >= fresh_take * 1000.0
            {
                return;
            }
            // A gap under liquid closes from above; pulling liquid in from the
            // side would only carry the gap away through the body. But a gap
            // over liquid that is draining away beneath it is where the pool
            // is emptying: the liquid around closes in from the sides too, so
            // an opening drains the pool as a funnel.
            if grounded
                && self.open_for_liquid(next)
                && self
                    .get(GridPosition {
                        x: next.x,
                        y: next.y + 1,
                    })
                    .is_some_and(TerrainKind::is_liquid)
                && !self.draining_under(next)
            {
                return;
            }
            if !self.fluid_destination(*cell, next, kind, player) {
                let above = GridPosition {
                    x: cell.x,
                    y: cell.y + 1,
                };
                if grounded && self.solid(next) && self.open_for_liquid(above) {
                    // A fast sheet runs up the wall it hits, then falls back.
                    self.flow[i].y = (self.flow[i].x.abs() - 1.0).max(0.0) * 0.45;
                }
                self.flow[i].x *= -0.2;
                self.phase[i].x = 0.0;
                return;
            }
            self.phase[i].x -= 1.0;
            self.swap_cells(*cell, next);
            *cell = next;
            i = self.index(next).unwrap();
            // Run off an edge instead of skating across the gap.
            if grounded
                && self.open_for_liquid(GridPosition {
                    x: cell.x,
                    y: cell.y - 1,
                })
            {
                break;
            }
        }
    }

    /// Transfer momentum to actual water and oil. Spray is made from pool cells,
    /// carries their heat/fuel, and deposits back into the material grid.
    pub(in crate::world) fn liquid_impulse(&mut self, impact: &LiquidDisturbance) {
        if self.cells.is_empty() || impact.strength <= 0.0 {
            return;
        }
        let radius = impact.radius.max(self.pixel_size);
        let min = self.world_to_cell(impact.start.min(impact.position) - Vec2::splat(radius));
        let max = self.world_to_cell(impact.start.max(impact.position) + Vec2::splat(radius));
        let delta = impact.position - impact.start;
        let spray_limit = (8.0 + impact.strength * 6.0) as usize;
        let mut sprayed = 0;
        let mut touched = false;
        // Top down, so a thrown surface droplet exposes the liquid under it and a
        // strong hit can open a small splash cavity.
        for y in (min.y.max(0)..=max.y.min(self.height - 1)).rev() {
            for x in min.x.max(0)..=max.x.min(self.width - 1) {
                let cell = GridPosition { x, y };
                if !self.get(cell).is_some_and(TerrainKind::is_liquid) {
                    continue;
                }
                let point = self.cell_to_world(cell);
                let t = ((point - impact.start).dot(delta) / delta.length_squared().max(0.001))
                    .clamp(0.0, 1.0);
                let offset = point - (impact.start + delta * t);
                if offset.length_squared() > radius * radius {
                    continue;
                }
                let origin = impact.start + delta * t;
                if self.sweep_circle(origin, point, 0.1).is_some() {
                    continue;
                }
                touched = true;
                self.wake(cell);
                let i = self.index(cell).unwrap();
                let influence = (1.0 - offset.length() / radius).max(0.15);
                if impact.heat > 0.0 {
                    self.temperature[i] =
                        (self.temperature[i] + impact.heat * influence).min(400.0);
                    self.track_heat(i);
                }
                let outward = offset.normalize_or_zero();
                let velocity = (impact.velocity * 0.24
                    + outward * impact.strength * 45.0
                    + Vec2::Y * impact.strength * 38.0)
                    * influence;
                self.flow[i] = (self.flow[i] + velocity / (self.pixel_size * 60.0))
                    .clamp(Vec2::splat(-MAX_CELL_SPEED), Vec2::splat(MAX_CELL_SPEED));
                let exposed = self
                    .get(GridPosition { x, y: y + 1 })
                    .is_none_or(TerrainKind::is_gas);
                if exposed
                    && !self.grain_at(GridPosition { x, y: y + 1 })
                    && sprayed < spray_limit
                    && velocity.length_squared() > 55.0 * 55.0
                    && !self.body_at(cell)
                {
                    // Each droplet gets its own angle and speed: a crown, not a fan.
                    let jitter = self.roll(cell, 12);
                    let angle = (jitter % 256) as f32 / 256.0 * 0.8 - 0.4;
                    let scale = 0.6 + ((jitter >> 8) % 256) as f32 / 256.0 * 0.7;
                    let launch = Vec2::new(
                        impact.velocity.x * 0.22 + outward.x * (80.0 + impact.strength * 35.0),
                        (impact.velocity.length() * 0.36 + impact.strength * 35.0)
                            .clamp(80.0, 320.0),
                    ) * influence.sqrt();
                    self.launch_droplet(cell, Vec2::from_angle(angle).rotate(launch) * scale);
                    sprayed += 1;
                }
            }
        }
        if touched {
            self.dirty = true;
        }
    }

    /// Something struck the surface near `point`: a few surface pixels leap
    /// up as droplets, more for a harder hit. Weak touches leave the pool be.
    pub(in crate::world) fn disturb_surface(&mut self, point: Vec2, strength: f32) {
        if self.cells.is_empty() || strength < 0.8 {
            return;
        }
        let center = self.world_to_cell(point);
        let count = (((strength - 0.8) * 1.6).ceil() as usize).min(4);
        let mut launched = 0;
        for dx in [0, 1, -1, 2, -2] {
            if launched >= count {
                break;
            }
            let Some(cell) = (-8..=10)
                .map(|dy| GridPosition {
                    x: center.x + dx,
                    y: center.y + dy,
                })
                .filter(|&cell| {
                    self.get(cell).is_some_and(TerrainKind::is_liquid)
                        && self.open_for_liquid(GridPosition {
                            x: cell.x,
                            y: cell.y + 1,
                        })
                })
                .min_by_key(|cell| (cell.y - center.y).abs())
            else {
                continue;
            };
            let dice = self.roll(cell, 46 + (dx + 2) as u32);
            let up = (50.0 + strength * 45.0) * (0.6 + (dice % 100) as f32 * 0.008);
            let sideways =
                (dx as f32 + ((dice >> 8) % 100) as f32 * 0.02 - 1.0) * (20.0 + strength * 25.0);
            self.launch_droplet(cell, Vec2::new(sideways, up));
            launched += 1;
        }
    }
}
