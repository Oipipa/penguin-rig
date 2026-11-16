//! Liquids and gases moving: each liquid cell's step (`liquid`), the
//! surface (`surface`), pressure through resting liquid (`pressure`) and
//! gas (`gas`), and the impulses shots leave in liquid.

use super::*;

mod gas;
mod liquid;
mod pressure;
mod surface;

#[derive(Message, Clone, Copy)]
pub struct LiquidDisturbance {
    pub position: Vec2,
    pub start: Vec2,
    pub radius: f32,
    pub velocity: Vec2,
    pub strength: f32,
    /// Degrees added to the liquid nearest the path (hot projectiles boil
    /// what they pass through); falls off toward the edge of `radius`.
    pub heat: f32,
}

/// Powder sinking through liquid moves on three ticks in four: slower than in air.
const SINK_CHANCE_IN_FOUR: u32 = 3;
/// Columns over which pressure moves surface liquid within an open pool.
const OPEN_PRESSURE_REACH: i32 = 24;
/// Falling liquid landing faster than this (cells/tick) sometimes splashes back.
const SPLASH_IMPACT: f32 = 1.8;
/// Chance per mille that an idle surface pixel with room beside it hops over:
/// Noita's pools are never quite still, so partial rows shimmer and thin
/// films creep, while full rows stay level.
const WANDER_WATER: u32 = 220;
const WANDER_OIL: u32 = 90;
const WANDER_BLOOD: u32 = 150;
/// Chance per mille per tick that a water-based pixel trades places with a
/// different water-based neighbor (blood or acid in water): stirred pools
/// mix pixel by pixel. Diffusion wakes nothing, so it stops when the pool rests.
const DIFFUSION: u32 = 45;
/// Depth of liquid (cells) at which a resting pixel beside a void is as
/// likely as not to be squeezed into it each tick. Deeper water gushes.
const SQUEEZE_DEPTH: i32 = 5;
/// Spread of gravity between falling pixels (a share either side of the
/// mean): no two drops fall quite alike, so a released body tears into
/// streaks instead of dropping as a rigid block.
const FALL_SCATTER: f32 = 0.2;
/// Chance per mille per tick that a falling pixel with open air beside it
/// drifts off sideways: the edges of a falling sheet fray.
const FRAY: u32 = 45;
/// Chance per mille per tick that a pixel leading a fast-falling body tears
/// off as a droplet of its own.
const TEAR: u32 = 40;

impl LevelGrid {
    /// Open space for liquid: empty or gas, and not held by a body or grain.
    fn open_for_liquid(&self, to: GridPosition) -> bool {
        let Some(j) = self.index(to) else {
            return false;
        };
        self.airborne[j].is_none()
            && self.cells[j].is_none_or(TerrainKind::is_gas)
            && !self.body_at(to)
    }

    /// Where a liquid cell may move: open space, down or sideways through a
    /// lighter liquid, or up through powder sinking into it.
    fn fluid_destination(
        &self,
        from: GridPosition,
        to: GridPosition,
        liquid: TerrainKind,
        player: Option<Vec2>,
    ) -> bool {
        let Some(j) = self.index(to) else {
            return false;
        };
        if self.airborne[j].is_some() || self.body_at(to) {
            return false;
        }
        match self.cells[j] {
            None => true,
            Some(kind) if kind.is_gas() => true,
            // Only a clearly heavier liquid moves through another as if it
            // were open space; near-equal ones trade places slowly, in
            // `step_liquid`, and mix on the way.
            Some(kind) if kind.is_liquid() && kind != liquid => {
                to.y <= from.y && liquid.sink_chance(kind) >= 1.0
            }
            Some(kind) if kind.is_powder() && to.y > from.y => {
                !self.blocked_by_player(from, player)
            }
            _ => false,
        }
    }

    /// Momentum decides which way a cell tries first; otherwise it is a coin toss,
    /// so resting liquid has no directional bias.
    fn preferred_side(&self, cell: GridPosition, flow: f32) -> i32 {
        if flow.abs() > 0.1 {
            flow.signum() as i32
        } else if self.roll(cell, 2) & 1 == 0 {
            1
        } else {
            -1
        }
    }

    /// Contiguous liquid resting on this cell, up to the depth that matters.
    fn liquid_load(&self, cell: GridPosition) -> i32 {
        (1..=24)
            .take_while(|dy| {
                self.get(GridPosition {
                    x: cell.x,
                    y: cell.y + dy,
                })
                .is_some_and(TerrainKind::is_liquid)
            })
            .count() as i32
    }

    /// The side on which an unloaded surface cell can glide over supported open
    /// cells and drop lower within reach; None when the surface here is level.
    fn surface_route(
        &self,
        cell: GridPosition,
        kind: TerrainKind,
        preferred: i32,
        player: Option<Vec2>,
    ) -> Option<i32> {
        let reach = kind.liquid_style().reach;
        let mut best: Option<(i32, i32)> = None;
        for side in [preferred, -preferred] {
            for distance in 1..=reach {
                if best.is_some_and(|(_, nearest)| distance >= nearest) {
                    break;
                }
                let beside = GridPosition {
                    x: cell.x + side * distance,
                    y: cell.y,
                };
                if !self.fluid_destination(cell, beside, kind, player) {
                    break;
                }
                let down = GridPosition {
                    x: beside.x,
                    y: cell.y - 1,
                };
                if self.fluid_destination(cell, down, kind, player) {
                    best = Some((side, distance));
                    break;
                }
            }
        }
        best.map(|(side, _)| side)
    }

    /// A liquid cell at rest under more of itself, with nothing open, falling
    /// or lighter below or beside it: `step_liquid` would change nothing, so
    /// the inside of an awake pool is skipped at the cost of a few reads.
    fn buried_idle(&self, cell: GridPosition, i: usize, kind: TerrainKind) -> bool {
        if cell.x <= 0 || cell.y <= 0 || cell.x >= self.width - 1 || cell.y >= self.height - 1 {
            return false;
        }
        if self.flow[i] != Vec2::ZERO || self.phase[i] != Vec2::ZERO {
            return false;
        }
        let at = |dx: i32, dy: i32| self.step_index(i, cell, dx, dy);
        let held = |j: Option<usize>| {
            j.is_some_and(|j| self.cells[j].is_some_and(|k| k == kind || k.is_solid()))
        };
        at(0, 1).is_some_and(|j| self.cells[j] == Some(kind))
            && held(at(-1, 0))
            && held(at(1, 0))
            && match at(0, -1) {
                Some(j) => match self.cells[j] {
                    Some(k) if k == kind => self.flow[j].y >= -0.05,
                    Some(k) => k.is_solid(),
                    None => false,
                },
                None => false,
            }
    }

    pub fn step_fluids(&mut self, player: Option<Vec2>) {
        // Bottom-up, alternating x order, over the awake chunks only.
        let forward = self.tick.is_multiple_of(2);
        crate::timed!(
            "  sim liquid cells",
            self.for_awake_cells(forward, sleep::AwakeMaterial::Liquid, |grid, cell, i| {
                if grid.updated[i] == grid.stamp() {
                    return;
                }
                let Some(kind) = grid.cells[i].filter(|kind| kind.is_liquid()) else {
                    return;
                };
                if grid.buried_idle(cell, i, kind) {
                    return;
                }
                let end = grid.step_liquid(cell, kind, player);
                // Momentum left over means it moves again soon.
                if let Some(j) = grid.index(end)
                    && grid.cells[j].is_some()
                    && (grid.flow[j] != Vec2::ZERO || grid.phase[j] != Vec2::ZERO)
                {
                    grid.keep_awake(end);
                }
            })
        );
        crate::timed!("  sim pressure", self.equalize_pressure(player));
        #[cfg(test)]
        let gases_started = std::time::Instant::now();
        let mut gases = Vec::new();
        self.active
            .gases
            .gather(&self.chunks, self.sleep.live(), &mut gases);
        // Top row first, right to left.
        gases.sort_unstable_by_key(|&i| {
            let cell = self.cell_of(i);
            std::cmp::Reverse((cell.y, cell.x))
        });
        for i in gases {
            if self.updated[i] == self.stamp() {
                continue;
            }
            if let Some(kind) = self.cells[i].filter(|kind| kind.is_gas()) {
                self.step_gas(self.cell_of(i), kind);
            }
        }
        #[cfg(test)]
        crate::dev::profile::add("  sim gases", gases_started.elapsed());
    }
}

pub(super) fn receive_disturbances(
    mut impacts: MessageReader<LiquidDisturbance>,
    mut grid: ResMut<LevelGrid>,
) {
    for impact in impacts.read() {
        grid.liquid_impulse(impact);
    }
}
