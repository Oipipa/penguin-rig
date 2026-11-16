//! Terrain as bodies meet it: what is solid, whether a box overlaps it,
//! and sweeping a circle through it to the first contact.

use super::*;

impl LevelGrid {
    /// A moving body can scatter a few disconnected chips in front of it.
    /// Eight-way connectivity keeps a corner of a wall from being mistaken
    /// for a loose pixel. Powder is counted separately from the firm floor
    /// it rests on. Larger pieces and bedrock remain real obstacles.
    pub fn brush_aside_fragments(&mut self, center: Vec2, half: Vec2, travel: Vec2) {
        const MAX_FRAGMENT: usize = 6;
        const MAX_BRUSHED: usize = 24;
        if travel.length_squared() < 0.0001 || self.cells.is_empty() {
            return;
        }
        let horizontal = travel.x.abs() >= travel.y.abs();
        let side = if horizontal {
            travel.x.signum()
        } else {
            travel.y.signum()
        };
        let (start, end) = if horizontal {
            (
                self.world_to_cell(center + Vec2::new(side * half.x, -half.y + 0.01)),
                self.world_to_cell(center + Vec2::new(travel.x + side * half.x, half.y - 0.01)),
            )
        } else {
            (
                self.world_to_cell(center + Vec2::new(-half.x + 0.01, side * half.y)),
                self.world_to_cell(center + Vec2::new(half.x - 0.01, travel.y + side * half.y)),
            )
        };
        let (left, right) = (start.x.min(end.x), start.x.max(end.x));
        let (bottom, top) = (start.y.min(end.y), start.y.max(end.y));
        let mut brushed = 0;
        let mut examined = Vec::new();
        for x in left..=right {
            for y in bottom..=top {
                if brushed >= MAX_BRUSHED {
                    return;
                }
                let seed = GridPosition { x, y };
                if examined.contains(&seed) || !self.get(seed).is_some_and(TerrainKind::is_solid) {
                    continue;
                }
                let mut piece = vec![seed];
                let mut cursor = 0;
                let mut anchored = false;
                while cursor < piece.len() && !anchored {
                    let cell = piece[cursor];
                    let Some(kind) = self.get(cell) else {
                        anchored = true;
                        break;
                    };
                    if kind == TerrainKind::Bedrock
                        || self.grain_at(cell)
                        || self.body_at(cell)
                        || piece.len() > MAX_FRAGMENT
                    {
                        anchored = true;
                        break;
                    }
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            if dx == 0 && dy == 0 {
                                continue;
                            }
                            let near = GridPosition {
                                x: cell.x + dx,
                                y: cell.y + dy,
                            };
                            let connected = if kind.is_powder() {
                                self.get(near).is_some_and(TerrainKind::is_powder)
                            } else {
                                self.solid(near)
                                    && !self.get(near).is_some_and(TerrainKind::is_powder)
                            };
                            if connected && !piece.contains(&near) {
                                piece.push(near);
                                if piece.len() > MAX_FRAGMENT {
                                    anchored = true;
                                    break;
                                }
                            }
                        }
                        if anchored {
                            break;
                        }
                    }
                    cursor += 1;
                }
                examined.extend_from_slice(&piece);
                if anchored || brushed + piece.len() > MAX_BRUSHED {
                    continue;
                }
                let chips: Vec<_> = piece
                    .iter()
                    .filter_map(|&cell| {
                        let i = self.index(cell)?;
                        let kind = self.cells[i]?;
                        Some((
                            cell,
                            kind.loose(),
                            self.loose_color(i, cell),
                            self.temperature[i],
                        ))
                    })
                    .collect();
                if chips.len() != piece.len() {
                    continue;
                }
                for &(cell, _, _, _) in &chips {
                    self.put(cell, None);
                }
                for (cell, kind, color, temperature) in chips {
                    let point = self.cell_to_world(cell);
                    let velocity = if horizontal {
                        Vec2::new(side * 105.0, 45.0 + (cell.y % 3) as f32 * 15.0)
                    } else {
                        let away = if point.x < center.x { -1.0 } else { 1.0 };
                        Vec2::new(away * 105.0, 45.0 + side * 35.0)
                    };
                    if self.launch_debris(point, velocity, kind, color, 4) {
                        if let Some(grain) = self.grains.last_mut() {
                            grain.temperature = temperature;
                        }
                    } else {
                        self.put(cell, Some(kind));
                    }
                    brushed += 1;
                }
            }
        }
    }

    /// Loose debris a moving body wades through (glass shards, scrap,
    /// rust, bone, mushroom and silk, flesh): every cell of it inside the
    /// body or just ahead of it (never under its feet) is kicked up and along
    /// the way it goes, so a heap of it is pushed about and scattered under
    /// foot, as sand is in Noita, rather than standing like rock.
    pub fn stir_loose(&mut self, center: Vec2, half: Vec2, velocity: Vec2) {
        const MOST: usize = 36;
        if self.cells.is_empty() || velocity.length_squared() < 30.0 * 30.0 {
            return;
        }
        let cell = self.pixel_size;
        let ahead = Vec2::new(velocity.x.signum() * cell * 1.5, 0.0);
        // Never what it stands on: only what is in its way, from its soles
        // up (a heap is pushed through, not dug into).
        let min = self
            .world_to_cell(center - half + Vec2::new(-cell, cell * 0.25) + ahead.min(Vec2::ZERO));
        let max = self.world_to_cell(center + half + Vec2::new(cell, 0.0) + ahead.max(Vec2::ZERO));
        let mut kicked = 0;
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                if kicked >= MOST {
                    return;
                }
                let at = GridPosition { x, y };
                let Some(i) = self.index(at) else {
                    continue;
                };
                let Some(kind) = self.cells[i] else {
                    continue;
                };
                if !kind.stirs() {
                    continue;
                }
                // Only what lies open to the air (or is inside the body):
                // a deep heap is worn down from its top, not dug through.
                let above = GridPosition { x, y: y + 1 };
                let inside = (self.cell_to_world(at) - center).abs().cmple(half).all();
                if !inside && self.get(above).is_some_and(|k| !k.is_gas()) {
                    continue;
                }
                let color = self.loose_color(i, at);
                let temperature = self.temperature[i];
                self.put(at, None);
                let dice = crate::world::materials::noise(x ^ self.tick as i32, y);
                let spread = ((dice % 100) as f32 / 100.0 - 0.5) * 60.0;
                let lift = 30.0 + ((dice >> 8) % 60) as f32 + velocity.x.abs() * 0.25;
                let push = Vec2::new(velocity.x * 0.75 + spread, lift + velocity.y.max(0.0) * 0.3);
                if self.launch_debris(self.cell_to_world(at), push, kind, color, 3) {
                    if let Some(grain) = self.grains.last_mut() {
                        grain.temperature = temperature;
                    }
                    kicked += 1;
                } else {
                    self.put(at, Some(kind));
                }
            }
        }
    }

    pub(super) fn solid(&self, pos: GridPosition) -> bool {
        // Closed sides and floor; the top remains open for airborne material.
        // A chunk packed away in the store is as solid as bedrock until the
        // live window reaches it.
        pos.x < 0
            || pos.x >= self.width
            || pos.y < 0
            || self.get(pos).is_some_and(TerrainKind::is_solid)
            || self.chunks.is_stored(pos.x, pos.y)
    }

    /// Whether a body cannot be at `pos`: solid, past the sides or floor,
    /// or packed away (for finding ways round the level).
    pub fn blocks(&self, pos: GridPosition) -> bool {
        self.solid(pos)
    }

    pub fn overlaps_box(&self, center: Vec2, half: Vec2) -> bool {
        if self.cells.is_empty() {
            return false;
        }
        // Touching a surface is not penetration.
        let min = self.world_to_cell(center - half + Vec2::splat(0.001));
        let max = self.world_to_cell(center + half - Vec2::splat(0.001));
        (min.y..=max.y).any(|y| (min.x..=max.x).any(|x| self.solid(GridPosition { x, y })))
    }

    pub(super) fn circle_normal(
        &self,
        center: Vec2,
        radius: f32,
        liquids: bool,
        grains: bool,
    ) -> Option<Vec2> {
        if self.cells.is_empty() {
            return None;
        }
        let min = self.world_to_cell(center - Vec2::splat(radius));
        let max = self.world_to_cell(center + Vec2::splat(radius));
        let half = Vec2::splat(self.pixel_size * 0.5);
        let mut normal = Vec2::ZERO;
        let mut hit = false;
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let cell = GridPosition { x, y };
                let grain_here = grains
                    && self
                        .index(cell)
                        .and_then(|i| self.airborne[i])
                        .is_some_and(|kind| liquids || kind.is_solid());
                if !self.solid(cell)
                    && !grain_here
                    && !(liquids
                        && (self.get(cell).is_some_and(TerrainKind::is_liquid)
                            || self.body_at(cell)))
                {
                    continue;
                }
                let local = center - self.cell_to_world(cell);
                let delta = local - local.clamp(-half, half);
                if delta.length_squared() < radius * radius {
                    hit = true;
                    normal += if delta.length_squared() > 0.00001 {
                        delta.normalize() * (radius - delta.length())
                    } else if local.x.abs() > local.y.abs() {
                        Vec2::X * local.x.signum()
                    } else {
                        Vec2::Y * local.y.signum()
                    };
                }
            }
        }
        hit.then(|| normal.normalize_or_zero())
    }

    /// Sweep the whole projectile, including its radius, in subpixel increments.
    pub fn sweep_circle(&self, start: Vec2, end: Vec2, radius: f32) -> Option<TerrainHit> {
        self.sweep_material(start, end, radius, false, true)
    }

    /// As `sweep_circle`, but through grains in flight: for a body big
    /// enough to bowl through them.
    pub fn sweep_body(&self, start: Vec2, end: Vec2, radius: f32) -> Option<TerrainHit> {
        self.sweep_material(start, end, radius, false, false)
    }

    pub(super) fn sweep_material(
        &self,
        start: Vec2,
        end: Vec2,
        radius: f32,
        liquids: bool,
        grains: bool,
    ) -> Option<TerrainHit> {
        if self.cells.is_empty() {
            return None;
        }
        let delta = end - start;
        let steps = (delta.length() / (self.pixel_size * 0.5)).ceil().max(1.0) as usize;
        for step in 0..=steps {
            let fraction = step as f32 / steps as f32;
            let point = start + delta * fraction;
            if let Some(mut normal) = self.circle_normal(point, radius, liquids, grains) {
                let mut low = step.saturating_sub(1) as f32 / steps as f32;
                let mut high = fraction;
                for _ in 0..8 {
                    let mid = (low + high) * 0.5;
                    if self
                        .circle_normal(start + delta * mid, radius, liquids, grains)
                        .is_some()
                    {
                        high = mid;
                    } else {
                        low = mid;
                    }
                }
                // Use the normal at the refined contact, not at the later,
                // penetrating sample. The latter catches neighboring pixel
                // corners and kicks a rolling casing sideways on a flat floor.
                normal = self
                    .circle_normal(start + delta * high, radius, liquids, grains)
                    .unwrap_or(normal);
                if normal == Vec2::ZERO {
                    normal = (-delta).normalize_or_zero();
                    if normal == Vec2::ZERO {
                        normal = Vec2::Y;
                    }
                }
                return Some(TerrainHit {
                    position: start + delta * low,
                    normal,
                    fraction: low,
                });
            }
        }
        None
    }
}
