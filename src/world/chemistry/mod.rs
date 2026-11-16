//! Heat and what it does: conduction and cooling, burning and fuel,
//! melting, boiling and freezing, and blast pressure breaking rock.

use super::*;

/// Snow turns to water a little above the cold it lies in.
pub const SNOW_MELTS: f32 = 28.0;
use crate::world::materials::BOILING_POINT;
use std::collections::BinaryHeap;

/// Share of the gap to ambient that a heated cell with an open face loses
/// each tick.
const AMBIENT_COOLING: f32 = 0.005;
/// Share of the gap to ambient that a buried cell loses each tick. The world
/// is a slice of something deeper: heat also soaks away into the rock behind
/// and in front of it. Without this, every blast's heat spread through the
/// rock forever, and the warm halo it left kept growing (and costing) until
/// it was thinner than `HEAT_EPSILON`, which took minutes.
pub(super) const BURIED_COOLING: f32 = 0.003;
/// Share of a quenched flame's heat that goes into the water that put it out.
const QUENCH_HEAT: f32 = 0.1;
/// A hot flame sometimes flashes one touching water-based pixel into steam
/// as it is quenched. The contact remains finite and the steam can condense.
const QUENCH_STEAM_CHANCE: u32 = 22;
/// A burning solid throws a spark on roughly one tick in this many.
const EMBER_ODDS: u32 = 80;
/// Blasts at least this hot leave a fireball in the air they fill.
const FIREBALL_HEAT: f32 = 400.0;
/// Fresh blast debris passes through its neighbors for this many ticks.
const EJECTA_GHOST_TICKS: u8 = 6;
/// Chance (percent) per tick that a flame lights fumes touching it.
const FUMES_CATCH: u32 = 45;
/// Chance per mille that open air at a fireball's edge fills with smoke.
const BLAST_SMOKE: u32 = 90;

impl LevelGrid {
    pub fn material_at(&self, point: Vec2) -> Option<TerrainKind> {
        if self.cells.is_empty() {
            return None;
        }
        self.get(self.world_to_cell(point))
    }

    pub fn temperature_at(&self, point: Vec2) -> f32 {
        if self.cells.is_empty() {
            return 20.0;
        }
        self.index(self.world_to_cell(point))
            .map_or(20.0, |i| self.temperature[i])
    }

    pub(super) fn swap_cells(&mut self, from: GridPosition, to: GridPosition) {
        let (Some(a), Some(b)) = (self.index(from), self.index(to)) else {
            return;
        };
        if self.cells[a].is_some_and(TerrainKind::is_solid)
            || self.cells[b].is_some_and(TerrainKind::is_solid)
        {
            self.render_cache.mark(&self.chunks, from);
            self.render_cache.mark(&self.chunks, to);
        }
        let occludes = render::occludes;
        let (from_blocks, to_blocks) = (
            self.cells[a].is_some_and(occludes),
            self.cells[b].is_some_and(occludes),
        );
        if from_blocks != to_blocks {
            self.render_cache
                .note_occluder(&self.chunks, from, to_blocks);
            self.render_cache
                .note_occluder(&self.chunks, to, from_blocks);
        }
        // Moving material keeps its own shade, so flow and falling grains show
        // visible texture rather than sliding through a fixed pattern.
        self.cells.swap(a, b);
        self.base_colors.swap(a, b);
        // Whatever moved wears the shade of where it came from.
        self.carried[a] = self.cells[a].is_some();
        self.carried[b] = self.cells[b].is_some();
        self.temperature.swap(a, b);
        self.life.swap(a, b);
        self.damage.swap(a, b);
        self.stain.swap(a, b);
        self.flow.swap(a, b);
        self.phase.swap(a, b);
        self.refresh_activity(a);
        self.refresh_activity(b);
        self.refresh_reaction_neighbors(from);
        self.refresh_reaction_neighbors(to);
        self.updated[a] = self.stamp();
        self.updated[b] = self.stamp();
        self.dirty = true;
        // Liquids mixing by diffusion must not keep their chunk awake by
        // themselves: they mix while the pool is stirred and rest with it.
        if !self.diffusing {
            self.wake(from);
            self.wake(to);
        }
    }

    pub fn add_heat(&mut self, point: Vec2, degrees: f32) {
        if self.cells.is_empty() {
            return;
        }
        let cell = self.world_to_cell(point);
        if let Some(index) = self.index(cell)
            && self.cells[index].is_some()
        {
            self.temperature[index] = (self.temperature[index] + degrees).min(1400.0);
            self.track_heat(index);
            self.dirty = true;
        }
    }

    pub fn step_chemistry(&mut self) {
        // Heat only moves in the live window; beyond it, warm rock keeps its
        // temperature until the player comes back.
        let mut work = std::mem::take(&mut self.active.heat_work);
        work.clear();
        self.active
            .hot
            .gather(&self.chunks, self.sleep.live(), &mut work);
        for &i in &work {
            self.active.heat_seen[i] = true;
        }
        let seeds = work.len();
        for n in 0..seeds {
            let i = work[n];
            let cell = self.cell_of(i);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                if let Some(j) = self.step_index(i, cell, dx, dy)
                    && self.cells[j].is_some()
                    && !self.active.heat_seen[j]
                {
                    self.active.heat_seen[j] = true;
                    work.push(j);
                }
            }
        }
        for &i in &work {
            self.thermal_delta[i] = 0.0;
        }
        // Exchange energy once per edge using unchanged temperatures. Water has
        // more thermal mass than fuel, and a tiny flame cannot instantly boil it.
        for &i in &work {
            let Some(kind) = self.cells[i] else { continue };
            let cell = self.cell_of(i);
            // Only the right and upper edges are needed: every shared edge
            // is exchanged once, using the old temperatures on both sides.
            for (dx, dy) in [(1, 0), (0, 1)] {
                let Some(j) = self.step_index(i, cell, dx, dy) else {
                    continue;
                };
                let Some(other) = self.cells[j] else { continue };
                if !self.active.heat_seen[j] {
                    continue;
                }
                let a = kind.thermal();
                let b = other.thermal();
                let heat = (self.temperature[j] - self.temperature[i])
                    * a.conductivity.min(b.conductivity)
                    * 0.5;
                self.thermal_delta[i] += heat / a.capacity;
                self.thermal_delta[j] -= heat / b.capacity;
            }
        }
        for &i in &work {
            let Some(kind) = self.cells[i] else { continue };
            let cell = self.cell_of(i);
            self.temperature[i] =
                (self.temperature[i] + self.thermal_delta[i]).clamp(-30.0, 1400.0);
            let exposed = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .into_iter()
                .any(|(dx, dy)| {
                    let x = cell.x + dx;
                    let y = cell.y + dy;
                    (x as u32) < self.width as u32
                        && (y as u32) < self.height as u32
                        && self
                            .step_index(i, cell, dx, dy)
                            .is_some_and(|j| self.cells[j].is_none_or(TerrainKind::is_gas))
                });
            // Exposed faces exchange with ambient air; buried cells lose heat
            // more slowly, into the depth of the rock.
            let cooling = if exposed {
                AMBIENT_COOLING
            } else {
                BURIED_COOLING
            };
            self.temperature[i] += (20.0 - self.temperature[i]) * cooling;
            // Lava is molten material, not a finite flame. It stays hot while
            // present and continually heats water, air and neighboring rock.
            if kind == TerrainKind::Lava {
                self.temperature[i] = self.temperature[i].max(1000.0);
            }
            let temperature = self.temperature[i];
            if temperature > 25.0 {
                self.dirty = true;
            }
            if kind == TerrainKind::Lava
                && self
                    .get(GridPosition {
                        x: cell.x,
                        y: cell.y + 1,
                    })
                    .is_none()
                && self.roll(cell, 87).is_multiple_of(700)
            {
                let dice = self.roll(cell, 88);
                self.spark(
                    self.cell_to_world(cell) + Vec2::Y * self.pixel_size,
                    Vec2::new((dice % 51) as f32 - 25.0, 35.0 + ((dice >> 8) % 50) as f32),
                    Glow::Molten,
                    36.0 + ((dice >> 16) % 25) as f32,
                );
            }
            if kind == TerrainKind::Ice && temperature > 35.0 {
                self.change_phase(cell, TerrainKind::Water, (temperature - 35.0).max(5.0));
            } else if kind == TerrainKind::Snow && temperature > SNOW_MELTS {
                self.change_phase(
                    cell,
                    TerrainKind::Water,
                    (temperature - SNOW_MELTS).max(4.0),
                );
            } else if let Some((point, into)) = kind.melts()
                && temperature >= point
            {
                // Rock, iron and glass melt into the flow (see their rows'
                // `melts`). Blood-cooled VolcanicRock and the outer Bedrock
                // do not melt.
                self.change_phase(cell, into, temperature.max(1000.0));
            } else if let Some(vapor) = kind.vapor()
                && temperature >= BOILING_POINT
            {
                // Water and blood boil off as steam, acid as flammable fumes.
                self.change_phase(cell, vapor, (temperature - 80.0).max(100.0));
                self.hear(super::soundscape::Noise::Boil, cell, 1.0);
            } else if kind == TerrainKind::Steam && temperature < 85.0 {
                // Hysteresis prevents boiling/condensing flicker at a hot surface.
                self.change_phase(cell, TerrainKind::Water, temperature + 12.0);
            } else if kind == TerrainKind::Fire {
                self.hear(super::soundscape::Noise::Burning, cell, 1.0);
                // Flame touching fumes lights them; the fire runs through the
                // cloud from pocket to pocket.
                for next in Self::neighbors(cell) {
                    if self.get(next) == Some(TerrainKind::Fumes)
                        && self.roll(next, 59) % 100 < FUMES_CATCH
                    {
                        self.ignite_fumes(next);
                    }
                }
                // A flame contains finite energy; nearby water absorbs it and quenches it.
                let wet = self.quenching_neighbor(cell);
                if let Some(water) = wet {
                    let capacity = self.cells[water].map_or(4.0, |k| k.thermal().capacity);
                    self.temperature[water] +=
                        (temperature - 20.0).max(0.0) * QUENCH_HEAT / capacity;
                    self.track_heat(water);
                    if temperature >= 400.0 && self.roll(cell, 67) % 100 < QUENCH_STEAM_CHANCE {
                        let at = self.cell_of(water);
                        self.change_phase(at, TerrainKind::Steam, 125.0);
                        self.reaction_glint(at, [155, 220, 245]);
                    }
                    self.hear(super::soundscape::Noise::Quench, cell, 1.0);
                }
                if wet.is_some() || temperature < 160.0 {
                    self.put(cell, Some(TerrainKind::Smoke));
                    self.temperature[i] = temperature.min(90.0);
                }
            } else if kind == TerrainKind::Fumes && temperature >= kind.ignition() {
                self.ignite_fumes(cell);
            } else if temperature >= kind.ignition() {
                let wet = self.quenching_neighbor(cell);
                if let Some(water) = wet {
                    let capacity = self.cells[water].map_or(4.0, |k| k.thermal().capacity);
                    let cooling = (temperature - self.temperature[water]).max(0.0) * 0.16;
                    self.temperature[i] -= cooling;
                    self.temperature[water] += cooling * kind.thermal().capacity / capacity;
                    self.track_heat(water);
                    continue;
                }
                let air = Self::neighbors(cell).any(|p| {
                    self.index(p).is_some()
                        && matches!(
                            self.get(p),
                            None | Some(TerrainKind::Fire | TerrainKind::Smoke)
                        )
                });
                // Buried fuel conducts heat, but combustion needs an exposed face.
                if !air {
                    continue;
                }
                self.life[i] = self.life[i].saturating_sub(1);
                if kind.is_solid() {
                    self.render_cache.mark_color(&self.chunks, cell);
                }
                if self.life[i] == 0 {
                    self.put(
                        cell,
                        Some(if kind == TerrainKind::Oil {
                            TerrainKind::Smoke
                        } else {
                            TerrainKind::Ash
                        }),
                    );
                    self.temperature[i] = 160.0;
                    continue;
                }
                self.temperature[i] = (temperature + 18.0).min(850.0);
                self.burn(cell, kind);
                self.dirty = true;
            }
        }
        for &i in &work {
            if (self.temperature[i] - 20.0).abs() <= activity::HEAT_EPSILON {
                self.temperature[i] = 20.0;
            }
            self.track_heat(i);
            self.active.heat_seen[i] = false;
        }
        self.active.heat_work = work;
    }

    /// A pocket of fumes flashes into flame.
    fn ignite_fumes(&mut self, cell: GridPosition) {
        let Some(i) = self.index(cell) else { return };
        self.put(cell, Some(TerrainKind::Fire));
        self.hear(super::soundscape::Noise::Ignite, cell, 1.0);
        self.temperature[i] = 780.0;
        self.life[i] = 12 + (self.roll(cell, 58) % 10) as u16;
        self.track_heat(i);
    }

    /// A neighboring cell of water or blood, which smothers flame.
    fn quenching_neighbor(&self, cell: GridPosition) -> Option<usize> {
        Self::neighbors(cell)
            .find(|&p| self.get(p).is_some_and(TerrainKind::quenches))
            .and_then(|p| self.index(p))
    }

    /// Material changes keep momentum and heat instead of resetting to a newly
    /// painted cell. Fuel/lifetime and render/activity bookkeeping come from put.
    pub(super) fn change_phase(&mut self, cell: GridPosition, kind: TerrainKind, temperature: f32) {
        let i = self.index(cell).unwrap();
        let velocity = self.flow[i];
        self.put(cell, Some(kind));
        self.flow[i] = velocity * if kind.is_gas() { 0.35 } else { 0.6 };
        self.temperature[i] = temperature;
        self.track_heat(i);
    }

    /// Detached material still reacts. Vapor returns to the grid immediately;
    /// burning chips retain their char and remaining fuel when they land.
    pub(super) fn react_grain(&mut self, grain: &mut Grain, player: Option<Vec2>) -> bool {
        if grain.kind == TerrainKind::Ice && grain.temperature > 35.0
            || grain.kind == TerrainKind::Snow && grain.temperature > SNOW_MELTS
        {
            grain.kind = TerrainKind::Water;
            grain.temperature = (grain.temperature - 30.0).max(5.0);
            grain.color = TerrainKind::Water.color(0, 0);
        }
        let cell = self.world_to_cell(grain.position);
        if let Some(vapor) = grain.kind.vapor()
            && grain.temperature >= BOILING_POINT
            && self.free_for_material(cell, player)
        {
            self.put(cell, Some(vapor));
            let i = self.index(cell).unwrap();
            self.temperature[i] = (grain.temperature - 80.0).max(100.0);
            self.flow[i] = grain.velocity / (self.pixel_size * 60.0) * 0.35;
            self.track_heat(i);
            return true;
        }
        if grain.temperature >= grain.kind.ignition() && self.free_for_material(cell, player) {
            grain.fuel = grain.fuel.saturating_sub(1);
            grain.temperature = (grain.temperature + 10.0).min(850.0);
            if grain.fuel == 0 {
                if grain.kind == TerrainKind::Oil {
                    self.put(cell, Some(TerrainKind::Smoke));
                    return true;
                }
                grain.kind = TerrainKind::Ash;
                grain.color = TerrainKind::Ash.color(cell.x, cell.y);
            } else if self.roll(cell, 33).is_multiple_of(5) {
                self.put(cell, Some(TerrainKind::Fire));
            }
        }
        false
    }

    /// A burning cell sheds tongues of flame into open air around it and, now and
    /// then, a spark. Chances rather than fixed periods keep flames from pulsing.
    fn burn(&mut self, cell: GridPosition, kind: TerrainKind) {
        let i = self.index(cell).unwrap();
        let flame_chance = match kind {
            TerrainKind::Oil => 65,
            TerrainKind::Wood => 55,
            _ => 35,
        };
        if self.roll(cell, 5) % 100 < flame_chance {
            let side = if self.roll(cell, 6) & 1 == 0 { 1 } else { -1 };
            for (dx, dy) in [(0, 1), (side, 0), (-side, 0), (0, -1)] {
                let target = GridPosition {
                    x: cell.x + dx,
                    y: cell.y + dy,
                };
                if self.index(target).is_some()
                    && matches!(
                        self.get(target),
                        None | Some(TerrainKind::Fire | TerrainKind::Smoke)
                    )
                    && !self.grain_at(target)
                {
                    let j = self.index(target).unwrap();
                    if self.get(target) != Some(TerrainKind::Fire) {
                        self.put(target, Some(TerrainKind::Fire));
                    }
                    // Keep a bright attached base while the upper plume cools.
                    self.temperature[j] = (self.temperature[i] + 180.0).clamp(620.0, 900.0);
                    self.life[j] = self.life[j].max(18);
                    self.track_heat(j);
                    break;
                }
            }
        }
        if !kind.is_liquid()
            && self.roll(cell, 7).is_multiple_of(EMBER_ODDS)
            && self.motes.len() < 4096
        {
            let point = self.cell_to_world(cell) + Vec2::Y * self.pixel_size;
            if self.material_at(point).is_none_or(TerrainKind::is_gas) {
                let dice = self.roll(cell, 8);
                self.motes.push(particles::Mote {
                    position: point,
                    previous: point,
                    velocity: Vec2::new(
                        (dice % 97) as f32 - 48.0,
                        50.0 + (dice >> 8) as f32 % 70.0,
                    ),
                    kind: ParticleKind::Ember,
                    energy: 90.0 + (dice >> 16) as f32 % 60.0,
                    phase: 0.0,
                });
            }
        }
    }

    pub fn neighbors(cell: GridPosition) -> impl Iterator<Item = GridPosition> {
        [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .map(move |(x, y)| GridPosition {
                x: cell.x + x,
                y: cell.y + y,
            })
    }

    /// Hot blasts turn part of the open air they fill into short-lived flame,
    /// densest at the core. It is ordinary fire: it rises, heats, and smokes.
    fn ignite_blast_air(&mut self, i: usize, fraction: f32, heat: f32) {
        let cell = self.cell_of(i);
        if fraction < 0.3 || (self.roll(cell, 17) % 1000) as f32 >= (fraction - 0.3) * 1100.0 {
            // Past the flame, the fireball's edge rolls up as a pall of
            // smoke, as a Noita explosion does.
            if (0.06..0.55).contains(&fraction) && self.roll(cell, 18) % 1000 < BLAST_SMOKE {
                self.put(cell, Some(TerrainKind::Smoke));
                self.temperature[i] = 80.0;
                self.track_heat(i);
            }
            return;
        }
        self.put(cell, Some(TerrainKind::Fire));
        self.temperature[i] = 380.0 + heat * fraction * 0.5;
        self.life[i] = (self.life[i] as f32 * (0.4 + fraction)) as u16 + 1;
    }

    pub fn excavate(&mut self, impact: &TerrainImpact) {
        self.blast_budget = 0;
        if self.cells.is_empty() || impact.radius <= 0.0 {
            return;
        }
        let start = self.world_to_cell(impact.position);
        let Some(start_index) = self.index(start) else {
            return;
        };
        if self.get(start) == Some(TerrainKind::Bedrock) {
            return;
        }
        // Props in reach are thrown apart into what they are made of.
        if impact.impulse > 0.0 {
            self.props_blasted(
                impact.position,
                impact.radius,
                impact.impulse.clamp(80.0, 500.0),
            );
        }
        // Blasts light up their surroundings for a moment: a fireball flares
        // orange, a bolt strike flickers blue.
        if impact.heat >= FIREBALL_HEAT {
            self.flash(
                impact.position,
                Vec3::new(2.4, 1.3, 0.5),
                impact.radius * 2.6,
                24,
            );
        } else if impact.heat > 0.0 {
            self.flash(
                impact.position,
                Vec3::new(0.5, 0.9, 1.3) * (impact.heat / 360.0),
                34.0 + impact.radius * 2.0,
                7,
            );
        }
        let budget = (impact.radius * 25.0 + impact.impulse * 1.5) as u32;
        // Pressure never travels past the radius, so only the box around it
        // (and the last blast's box, to clear it) is ever touched.
        if let Some((min, max)) = self.blast_box.take() {
            for y in min.y..=max.y {
                for x in min.x..=max.x {
                    if let Some(i) = self.chunks.index(x, y) {
                        self.pressure[i] = 0;
                    }
                }
            }
        }
        let reach = (impact.radius / self.pixel_size).ceil() as i32 + 1;
        let (min, max) = (
            GridPosition {
                x: (start.x - reach).max(0),
                y: (start.y - reach).max(0),
            },
            GridPosition {
                x: (start.x + reach).min(self.width - 1),
                y: (start.y + reach).min(self.height - 1),
            },
        );
        self.blast_box = Some((min, max));
        self.pressure[start_index] = budget;
        let mut frontier = BinaryHeap::from([(budget, start_index)]);
        // Propagate on the intact field first. A wall absorbs pressure even if it
        // fractures this tick; destroying it never retroactively exposes its back.
        while let Some((energy, i)) = frontier.pop() {
            if self.pressure[i] != energy {
                continue;
            }
            let cell = self.cell_of(i);
            for (dx, dy) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (-1, 1),
                (1, -1),
                (-1, -1),
            ] {
                let next = GridPosition {
                    x: cell.x + dx,
                    y: cell.y + dy,
                };
                let Some(j) = self.index(next) else { continue };
                if self.cell_to_world(next).distance(impact.position) > impact.radius {
                    continue;
                }
                if dx != 0
                    && dy != 0
                    && (self.get(GridPosition {
                        x: cell.x + dx,
                        y: cell.y,
                    }) == Some(TerrainKind::Bedrock)
                        || self.get(GridPosition {
                            x: cell.x,
                            y: cell.y + dy,
                        }) == Some(TerrainKind::Bedrock))
                {
                    continue;
                }
                let travel =
                    (self.pixel_size * 25.0 * if dx != 0 && dy != 0 { 1.414 } else { 1.0 }) as u32;
                let resistance = self.cells[j].map_or(0, |kind| {
                    if kind == TerrainKind::Bedrock {
                        u32::MAX
                    } else {
                        (kind.blast_resistance() as f32 * self.pixel_size / 4.0) as u32
                    }
                });
                let next_energy = energy.saturating_sub(travel + resistance.min(u32::MAX - travel));
                if next_energy > self.pressure[j] {
                    self.pressure[j] = next_energy;
                    frontier.push((next_energy, j));
                }
            }
        }
        self.blast_budget = budget;
        let chunks = &self.chunks;
        let cells: Vec<usize> = (min.y..=max.y)
            .flat_map(|y| (min.x..=max.x).filter_map(move |x| chunks.index(x, y)))
            .collect();
        for i in cells {
            let energy = self.pressure[i];
            if energy == 0 {
                continue;
            }
            let fraction = energy as f32 / budget.max(1) as f32;
            let Some(kind) = self.cells[i] else {
                if impact.heat >= FIREBALL_HEAT && self.airborne[i].is_none() {
                    self.ignite_blast_air(i, fraction, impact.heat);
                }
                continue;
            };
            if kind == TerrainKind::Bedrock {
                continue;
            }
            self.temperature[i] = (self.temperature[i] + impact.heat * fraction).min(1400.0);
            self.damage[i] += energy as f32;
            self.render_cache.mark_color(&self.chunks, self.cell_of(i));
            self.track_heat(i);
            self.dirty = true;
            if kind.is_gas() || self.damage[i] < kind.toughness() {
                continue;
            }
            let cell = self.cell_of(i);
            let position = self.cell_to_world(cell);
            let temperature = self.temperature[i];
            let fuel = self.life[i];
            let color = self.loose_color(i, cell);
            self.put(cell, None);
            self.reserve_grain(position, Some(kind.loose()));
            // Every fragment leaves on its own heading and speed; the cloud
            // should read as a burst, not a geometric fan.
            let dice = self.roll(cell, 16);
            let heading = ((dice % 1024) as f32 / 1024.0 - 0.5) * 0.7;
            let scale = 0.6 + ((dice >> 10) % 1024) as f32 / 1024.0 * 0.65;
            let direction = ((position - impact.position).normalize_or_zero() + Vec2::Y * 1.4)
                .normalize_or_zero();
            self.grains.push(Grain {
                position,
                velocity: Vec2::from_angle(heading).rotate(direction)
                    * impact.impulse
                    * (0.35 + fraction * 0.65)
                    * scale,
                kind: kind.loose(),
                temperature,
                fuel,
                color,
                ghost: EJECTA_GHOST_TICKS,
                rest: 0,
            });
        }
        for grain in &mut self.grains {
            let x = (grain.position.x / self.pixel_size + self.width as f32 * 0.5).floor() as i32;
            let y = (grain.position.y / self.pixel_size + self.height as f32 * 0.5).floor() as i32;
            let Some(i) = self.chunks.index(x, y) else {
                continue;
            };
            let fraction = self.pressure[i] as f32 / budget.max(1) as f32;
            grain.velocity += (grain.position - impact.position).normalize_or_zero()
                * impact.impulse
                * fraction
                * 0.2;
        }
        // Work outward from the exposed surface so a dense blast can expand.
        self.grains
            .sort_unstable_by(|a, b| b.position.y.total_cmp(&a.position.y));
    }
}
