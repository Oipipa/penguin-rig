//! Moving bodies meeting liquid: rigid things displacing it, characters
//! wading and splashing Noita style, exact immersion, and the grid's side
//! of the penguin's coat (taking, catching, trading and dripping pixels).

use super::*;
use std::collections::VecDeque;

/// Slower than this (world units per second), a character slips through
/// liquid without throwing any of it.
const WADE_SPEED: f32 = 90.0;

/// A body moving through the material this fixed tick. The sweep makes fast
/// entries disturb the surface even when the body finishes underwater.
///
/// Rigid things (a bomb casing) `displace` liquid: they hold their volume
/// clear of the grid. Characters do not, as in Noita: liquid flows around
/// and over them, and only their fast-moving leading edge throws the liquid
/// it meets into the air as droplets. A swimmer that held its volume clear
/// would carry water up on its head and leave holes behind it.
#[derive(Message, Clone, Copy)]
pub struct LiquidBody {
    pub start: Vec2,
    pub end: Vec2,
    pub half: Vec2,
    pub velocity: Vec2,
    pub round: bool,
    pub displaces: bool,
}

impl LiquidBody {
    fn contains(self, point: Vec2) -> bool {
        let local = (point - self.end) / self.half;
        if self.round {
            local.length_squared() < 1.0
        } else {
            local.abs().max_element() < 1.0
        }
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Immersion {
    /// Share of the body in any liquid; the rest break it down by kind.
    pub fraction: f32,
    pub oil: f32,
    pub acid: f32,
    pub blood: f32,
    pub lava: f32,
    pub current: Vec2,
}

impl Immersion {
    /// The share in plain water.
    pub fn water(&self) -> f32 {
        (self.fraction - self.oil - self.acid - self.blood - self.lava).max(0.0)
    }
}

impl LevelGrid {
    /// Whether a body holding its volume clear of liquid covers `cell`.
    pub(super) fn body_at(&self, cell: GridPosition) -> bool {
        self.bodies
            .iter()
            .any(|body| body.displaces && body.contains(self.cell_to_world(cell)))
    }

    /// Sample displaced volume from the wet boundary of the previous body, so
    /// clearing its interior does not switch buoyancy off on the following tick.
    fn liquid_sample_at(&self, point: Vec2) -> Option<(TerrainKind, Vec2, usize)> {
        let at = self.world_to_cell(point);
        if let Some(kind) = self.get(at).filter(|kind| kind.is_liquid()) {
            let i = self.index(at)?;
            return Some((kind, self.flow[i] * self.pixel_size * 60.0, i));
        }
        for body in &self.bodies {
            if !body.displaces || !body.contains(point) {
                continue;
            }
            for side in [-1.0, 1.0] {
                let edge = Vec2::new(body.end.x + side * (body.half.x + self.pixel_size), point.y);
                let at = self.world_to_cell(edge);
                if let Some(kind) = self.get(at).filter(|kind| kind.is_liquid()) {
                    let i = self.index(at)?;
                    return Some((kind, self.flow[i] * self.pixel_size * 60.0, i));
                }
            }
        }
        None
    }

    /// How much of a body (box, or ellipse when `round`) is in liquid: the
    /// exact share of its height under liquid, averaged over five columns.
    /// It changes smoothly as the body moves, so buoyancy settles instead of
    /// stepping between sample rows and bobbing forever.
    pub fn immersion(&self, center: Vec2, half: Vec2, round: bool) -> Immersion {
        const COLUMNS: usize = 5;
        let mut result = Immersion::default();
        let mut total = 0.0;
        if self.cells.is_empty() {
            return result;
        }
        for column in 0..COLUMNS {
            let across = (column as f32 + 0.5) / COLUMNS as f32 * 2.0 - 1.0;
            let reach = if round {
                (1.0 - across * across).max(0.0).sqrt()
            } else {
                1.0
            };
            let x = center.x + across * half.x;
            let (y0, y1) = (center.y - reach * half.y, center.y + reach * half.y);
            if y1 <= y0 {
                continue;
            }
            total += y1 - y0;
            let mut cell = self.world_to_cell(Vec2::new(x, y0));
            loop {
                let bottom = self.cell_to_world(cell).y - self.pixel_size * 0.5;
                let top = bottom + self.pixel_size;
                let overlap = top.min(y1) - bottom.max(y0);
                if overlap > 0.0
                    && let Some((kind, current, _)) =
                        self.liquid_sample_at(Vec2::new(x, (bottom + top) * 0.5))
                {
                    result.fraction += overlap;
                    match kind {
                        TerrainKind::Oil => result.oil += overlap,
                        TerrainKind::Acid => result.acid += overlap,
                        TerrainKind::Blood => result.blood += overlap,
                        TerrainKind::Lava => result.lava += overlap,
                        _ => {}
                    }
                    result.current += current * overlap;
                }
                if top >= y1 {
                    break;
                }
                cell.y += 1;
            }
        }
        if result.fraction > 0.0 {
            result.current /= result.fraction;
        }
        if total > 0.0 {
            result.fraction /= total;
            result.oil /= total;
            result.acid /= total;
            result.blood /= total;
            result.lava /= total;
        }
        result
    }

    /// A character moving through liquid, Noita style: it holds no volume,
    /// but the liquid its leading edge runs into at speed is thrown into the
    /// air as droplets and pushed along, so a dive raises a crown, a swimmer
    /// pushes a bow wave, and jumping out throws a little water up after it.
    fn wade(&mut self, body: LiquidBody) {
        let speed = body.velocity.length();
        if speed < WADE_SPEED {
            return;
        }
        let heading = body.velocity / speed;
        // The middle of the leading face, before and after the move.
        let lead = heading * body.half;
        // Plunging in throws the most; breaking out or pushing sideways
        // throws less, and nothing is carried along.
        let strength = if heading.y < -0.5 {
            (speed / 180.0).min(3.0)
        } else {
            (speed / 360.0).min(1.2)
        };
        self.liquid_impulse(&LiquidDisturbance {
            position: body.end + lead,
            start: body.start + lead,
            radius: body.half.x,
            velocity: body.velocity,
            strength,
            heat: 0.0,
        });
    }

    /// Take one pixel of `kind` from right at or under `cell`, for a body
    /// surfacing out of it to carry away as a film. Returns the pixel's own
    /// shade, or None when there is none within reach.
    pub fn take_liquid_near(&mut self, cell: GridPosition, kind: TerrainKind) -> Option<[u8; 4]> {
        if self.cells.is_empty() {
            return None;
        }
        // Under it first, then beside it (a body walking out of a pool
        // leaves it behind to one side).
        const NEAR: [(i32, i32); 16] = [
            (0, 0),
            (0, -1),
            (-1, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (0, -2),
            (-1, -2),
            (1, -2),
            (0, -3),
            (-2, 0),
            (2, 0),
            (-2, -1),
            (2, -1),
            (-3, 0),
            (3, 0),
        ];
        for (dx, dy) in NEAR {
            let at = GridPosition {
                x: cell.x + dx,
                y: cell.y + dy,
            };
            let Some(i) = self.index(at) else { continue };
            if self.cells[i] == Some(kind) && !self.grain_at(at) {
                let color = self.base_colors[i];
                self.put(at, None);
                return Some(color);
            }
        }
        None
    }

    /// Swap a held pixel of `kind` (with its `color`) for the pixel of other
    /// liquid at `cell`: a body washing off in a pool leaves what it carried
    /// there and takes the pool's liquid instead. Returns what it took.
    pub fn exchange_liquid(
        &mut self,
        cell: GridPosition,
        kind: TerrainKind,
        color: [u8; 4],
    ) -> Option<(TerrainKind, [u8; 4])> {
        let i = self.index(cell)?;
        let found = self.cells[i].filter(|found| found.is_liquid() && *found != kind)?;
        if self.grain_at(cell) {
            return None;
        }
        let taken = (found, self.base_colors[i]);
        self.transmute(cell, kind);
        self.base_colors[i] = color;
        self.carried[i] = true;
        Some(taken)
    }

    /// Turn a pixel carried by a body into gas at `cell` (water boiling off
    /// as steam, burning oil as flame, acid fuming away). Returns false and
    /// keeps the pixel when the cell is taken.
    pub fn vent(&mut self, cell: GridPosition, gas: TerrainKind, temperature: f32) -> bool {
        let Some(i) = self.index(cell) else {
            return false;
        };
        if self.cells[i].is_some() || self.grain_at(cell) {
            return false;
        }
        self.put(cell, Some(gas));
        self.temperature[i] = temperature;
        self.track_heat(i);
        true
    }

    /// Airborne liquid drops inside `min..max`: `take` sees each one's
    /// position, velocity, kind and shade and says whether a body catches
    /// it. Caught drops leave the air: the body now carries them.
    pub fn catch_drops(
        &mut self,
        min: Vec2,
        max: Vec2,
        mut take: impl FnMut(Vec2, Vec2, TerrainKind, [u8; 4]) -> bool,
    ) {
        let mut grains = std::mem::take(&mut self.grains);
        grains.retain(|grain| {
            let inside = grain.position.x >= min.x
                && grain.position.y >= min.y
                && grain.position.x <= max.x
                && grain.position.y <= max.y;
            if inside
                && grain.kind.is_liquid()
                && take(grain.position, grain.velocity, grain.kind, grain.color)
            {
                self.reserve_grain(grain.position, None);
                return false;
            }
            true
        });
        self.grains = grains;
    }

    /// Let one drop of clinging liquid fall from `position`. It is an
    /// ordinary droplet: it falls, lands and rejoins the grid. Returns false
    /// (and keeps the drop) when there is no room for it there.
    pub fn shed_drop(
        &mut self,
        position: Vec2,
        velocity: Vec2,
        kind: TerrainKind,
        color: [u8; 4],
    ) -> bool {
        self.launch_pixel(position, velocity, kind, color, 0)
    }

    /// Throw one fresh pixel of `kind` into the air at `position`: a drop
    /// from a wound, a burst bubble, a creature coming apart. `ghost` ticks
    /// let a packed burst pass through its own drops while it spreads out.
    /// Returns false when there is no room for it there.
    pub fn launch_liquid(
        &mut self,
        position: Vec2,
        velocity: Vec2,
        kind: TerrainKind,
        ghost: u8,
    ) -> bool {
        let cell = self.world_to_cell(position);
        self.launch_pixel(position, velocity, kind, kind.color(cell.x, cell.y), ghost)
    }

    /// Throw one pixel of solid debris of `kind` in the given color: what
    /// is left of a creature, say. A clump of it falls together, passing
    /// through itself while it falls (for `ghost` ticks), and piles up
    /// where it lands. Returns false when there is no room for it there.
    pub fn launch_debris(
        &mut self,
        position: Vec2,
        velocity: Vec2,
        kind: TerrainKind,
        color: [u8; 4],
        ghost: u8,
    ) -> bool {
        self.launch_pixel(position, velocity, kind, color, ghost)
    }

    /// `launch_debris`, hot: a burning or molten piece of something.
    pub(super) fn launch_hot(
        &mut self,
        position: Vec2,
        velocity: Vec2,
        kind: TerrainKind,
        color: [u8; 4],
        temperature: f32,
    ) -> bool {
        if !self.launch_pixel(position, velocity, kind, color, 6) {
            return false;
        }
        if let Some(grain) = self.grains.last_mut() {
            grain.temperature = temperature;
        }
        true
    }

    /// `launch_liquid` for a pixel that keeps its own shade.
    fn launch_pixel(
        &mut self,
        position: Vec2,
        velocity: Vec2,
        kind: TerrainKind,
        color: [u8; 4],
        ghost: u8,
    ) -> bool {
        let cell = self.world_to_cell(position);
        if self.cells.is_empty() || !self.free_for_material(cell, None) {
            return false;
        }
        self.grains.push(Grain {
            position,
            velocity,
            kind,
            temperature: 20.0,
            fuel: kind.lifetime(),
            color,
            ghost,
            rest: 0,
        });
        self.reserve_grain(position, Some(kind));
        self.dirty = true;
        true
    }

    pub(super) fn displace_body(&mut self, body: LiquidBody) {
        if self.cells.is_empty() {
            return;
        }
        let min = self.world_to_cell(body.end - body.half);
        let max = self.world_to_cell(body.end + body.half);
        let mut seeds = Vec::new();
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let cell = GridPosition { x, y };
                if body.contains(self.cell_to_world(cell))
                    && self.index(cell).is_some()
                    && !self.solid(cell)
                    && !self.grain_at(cell)
                {
                    seeds.push(cell);
                }
            }
        }
        let mut remaining: std::collections::HashSet<_> = seeds.iter().copied().collect();
        for seed in seeds {
            if !remaining.remove(&seed) {
                continue;
            }
            let mut region = vec![seed];
            let mut cursor = 0;
            while cursor < region.len() {
                for next in Self::neighbors(region[cursor]) {
                    if remaining.remove(&next) {
                        region.push(next);
                    }
                }
                cursor += 1;
            }
            // A wall can split a body's footprint. Each side needs its own
            // reachable vacancies; never pair sources across that wall.
            self.displace_body_region(body, region);
        }
    }

    fn displace_body_region(&mut self, body: LiquidBody, region: Vec<GridPosition>) {
        let sources: Vec<_> = region
            .iter()
            .copied()
            .filter(|&cell| self.get(cell).is_some_and(TerrainKind::is_liquid))
            .collect();
        let mut visited: std::collections::HashSet<_> = region.iter().copied().collect();
        let mut queue: VecDeque<_> = region.into();
        let mut targets = Vec::new();
        while targets.len() < sources.len() && visited.len() < 8192 {
            let Some(cell) = queue.pop_front() else { break };
            // Up first gives displaced water a crest instead of forcing it downward.
            for (dx, dy) in [(0, 1), (-1, 0), (1, 0), (0, -1)] {
                let next = GridPosition {
                    x: cell.x + dx,
                    y: cell.y + dy,
                };
                if self.index(next).is_none()
                    || self.solid(next)
                    || self.grain_at(next)
                    || !visited.insert(next)
                {
                    continue;
                }
                if !self.body_at(next) && self.get(next).is_none_or(TerrainKind::is_gas) {
                    targets.push(next);
                    if targets.len() == sources.len() {
                        break;
                    }
                }
                queue.push_back(next);
            }
        }
        let mut spray = Vec::new();
        for (from, to) in sources.into_iter().zip(targets) {
            self.swap_cells(from, to);
            let i = self.index(to).unwrap();
            let outward = (self.cell_to_world(to) - body.end).normalize_or_zero();
            let speed = body.velocity.length().min(500.0);
            self.flow[i] =
                (body.velocity * 0.18 + outward * speed * 0.15) / (self.pixel_size * 60.0);
            if speed > 110.0 && spray.len() < 10 {
                spray.push(to);
            }
        }
        for cell in spray {
            let above = GridPosition {
                x: cell.x,
                y: cell.y + 1,
            };
            if self.get(above).is_some_and(|kind| !kind.is_gas())
                || self.body_at(above)
                || self.grain_at(above)
            {
                continue;
            }
            let i = self.index(cell).unwrap();
            let position = self.cell_to_world(cell);
            let side = (position.x - body.end.x).signum();
            let speed = body.velocity.length().min(500.0);
            let jitter = self.roll(cell, 31);
            let scale = 0.65 + (jitter % 64) as f32 / 90.0;
            let velocity = Vec2::new(
                body.velocity.x * 0.2 + side * speed * 0.3,
                75.0 + speed * 0.4,
            );
            let grain = Grain {
                position: position + Vec2::Y * self.pixel_size * 0.9,
                velocity: Vec2::from_angle(((jitter >> 8) % 64) as f32 / 160.0 - 0.2)
                    .rotate(velocity)
                    * scale,
                kind: self.cells[i].unwrap(),
                temperature: self.temperature[i],
                fuel: self.life[i],
                color: self.base_colors[i],
                ghost: 0,
                rest: 0,
            };
            self.put(cell, None);
            self.reserve_grain(grain.position, Some(grain.kind));
            self.grains.push(grain);
        }
    }
}

pub(super) fn receive_bodies(mut bodies: MessageReader<LiquidBody>, mut grid: ResMut<LevelGrid>) {
    grid.bodies.clear();
    grid.bodies.extend(bodies.read().copied());
    for body in grid.bodies.clone() {
        if !body.displaces {
            grid.wade(body);
            continue;
        }
        // Only the leading moving edge creates spray. A resting body still excludes water.
        if body.velocity.length_squared() > 30.0 * 30.0 {
            let feet = Vec2::Y * (body.half.y - body.half.x).max(0.0);
            grid.liquid_impulse(&LiquidDisturbance {
                position: body.end - feet,
                start: body.start - feet,
                radius: body.half.x,
                velocity: body.velocity,
                strength: (body.velocity.length() / 180.0).min(3.0),
                heat: 0.0,
            });
        }
        grid.displace_body(body);
    }
}
