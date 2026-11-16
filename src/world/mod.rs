//! Cell storage and fixed-tick material simulation extracted from Penguin.
//!
//! Powders, fluids, chemistry, reactions, support, debris, and motes use
//! the original material rules. The editor draws cells separately.

#![allow(dead_code)] // Preserve material helpers unused by the sandbox UI.

use bevy::prelude::*;

use crate::Tuning;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GridPosition {
    pub x: i32,
    pub y: i32,
}

pub use materials::TerrainKind;
pub mod activity;
mod bodies;
mod chemistry;
pub mod chunks;
mod collision;
mod flow;
pub mod materials;
pub mod particles;
mod powders;
mod reactions;
pub mod render;
pub mod sleep;
mod slots;
pub mod soundscape;
mod store;
mod stream;
mod support;

pub use bodies::LiquidBody;
pub use flow::LiquidDisturbance;
pub use particles::{Glow, ParticleEmission, ParticleKind};
const PLAYER_HALF_SIZE: Vec2 = Vec2::new(13.0, 21.0);

#[derive(Clone, Copy, Debug)]
pub struct TerrainImpact {
    pub position: Vec2,
    pub radius: f32,
    pub impulse: f32,
    pub heat: f32,
}

/// Coarse arena generation metadata; collision and destruction use material pixels.
#[derive(Clone, Copy, Debug)]
pub struct TerrainTile {
    pub grid_position: GridPosition,
    pub kind: TerrainKind,
    pub destructible: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct TerrainHit {
    pub position: Vec2,
    pub normal: Vec2,
    pub fraction: f32,
}

/// A cell of snow taken up out of the world, to be put back (see
/// `LevelGrid::scoop_snow`): its warmth, how long it has lain, its shade.
#[derive(Clone, Copy, Debug)]
pub struct Lump {
    temperature: f32,
    life: u16,
    color: [u8; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct Grain {
    pub position: Vec2,
    pub velocity: Vec2,
    kind: TerrainKind,
    temperature: f32,
    fuel: u16,
    /// The pixel keeps its own shade in flight and after it lands.
    color: [u8; 4],
    /// Ticks during which freshly ejected debris ignores other airborne grains,
    /// so a packed blast cloud can expand instead of jamming against itself.
    ghost: u8,
    /// Ticks a grain wedged in a sealed pocket waits before it looks for room
    /// again (see `JAMMED_REST`).
    rest: u8,
}

/// Ticks a grain that found no room to settle or climb into waits before
/// searching again. Wedged debris costs nothing while it waits.
const JAMMED_REST: u8 = 30;
/// Cells a jammed grain looks up through a loose pile for open air.
const CLIMB_REACH: i32 = 256;

/// Gravity for material moving cell to cell, in cells per tick squared.
const FALL_ACCELERATION: f32 = 0.32;
/// Fastest a cell-grid material moves in one tick.
const MAX_CELL_SPEED: f32 = 4.0;

#[derive(Resource, Default)]
#[cfg_attr(test, derive(Clone))]
pub struct LevelGrid {
    /// Enables snowfall and wind around the visible grid.
    pub weather: bool,
    /// Which chunks are resident and in which slot of the per-cell arrays
    /// below (see `chunks.rs`). Every per-cell array holds
    /// `chunks.capacity()` slots of `CHUNK_CELLS` cells.
    chunks: chunks::ChunkMap,
    /// Chunks away from the live window, packed (see `store.rs`), by chunk
    /// number.
    store: std::collections::HashMap<usize, store::StoredChunk>,
    pub cells: Vec<Option<TerrainKind>>,
    pub active: activity::Activity,
    /// Which chunks the cell rules visit this tick.
    sleep: sleep::SimChunks,
    /// Set while a surface pixel idly wanders: its moves keep the pool
    /// stepping but do not ask the pressure pass to re-level it.
    quiet_moves: bool,
    /// Set while two miscible liquids trade places by diffusion: such swaps
    /// wake nothing.
    diffusing: bool,
    pub flow: Vec<Vec2>,
    phase: Vec<Vec2>,
    pub base_colors: Vec<[u8; 4]>,
    /// Whether a cell's color came from somewhere else (it flowed or fell
    /// here, or was painted on) rather than being its own material's shade
    /// at this spot, which can be worked out again from its kind and place.
    pub carried: Vec<bool>,
    /// Blood soaked into a solid (0 clean ..= MAX_STAIN). Painted over the
    /// material's own color; flowing water washes it out again.
    stain: Vec<u8>,
    render_cache: render::RenderCache,
    /// Cells watched by the editor. None simulates the whole grid.
    pub(crate) view: Option<render::CellRect>,
    /// Last view populated with snowflakes.
    snow_view: Option<render::CellRect>,
    pub temperature: Vec<f32>,
    thermal_delta: Vec<f32>,
    damage: Vec<f32>,
    life: Vec<u16>,
    /// Tick each cell last moved or changed on (the low 32 bits).
    updated: Vec<u32>,
    pressure: Vec<u32>,
    /// Cells the last blast's pressure could reach (all others hold 0), so
    /// the next blast clears and scans only that box, not the whole arena.
    blast_box: Option<(GridPosition, GridPosition)>,
    /// Pressure budget of the last excavation (0 when it delivered none), so
    /// bodies can read how much of the blast reached them.
    blast_budget: u32,
    /// Per-cell visit stamps for flood fills; a cell is visited this pass when
    /// its stamp equals `visit_stamp`, so nothing has to be cleared between passes.
    visited: Vec<u32>,
    visit_stamp: u32,
    pub motes: Vec<particles::Mote>,
    pub grains: Vec<Grain>,
    /// How many chunks across and down the live window spans, unclipped by
    /// the world's edges (see `update_live`).
    live_span: (i32, i32),
    /// Grains that were in the air when the live window left them, by
    /// chunk number. They keep their cells and wait there, costing
    /// nothing, until the window comes back over their chunk.
    parked: std::collections::HashMap<usize, Vec<Grain>>,
    parked_count: usize,
    pub airborne: Vec<Option<TerrainKind>>,
    /// Grains reserving each cell; ghosted debris may briefly overlap.
    airborne_count: Vec<u8>,
    bodies: Vec<LiquidBody>,
    pub support_dirty: Vec<GridPosition>,
    /// What the material sounded like since the audio last listened.
    soundscape: soundscape::Soundscape,
    pub width: i32,
    pub height: i32,
    pub pixel_size: f32,
    tick: u64,
    pub dirty: bool,
}

impl LevelGrid {
    pub fn new(tuning: &Tuning) -> Self {
        let mut grid = Self::unloaded(tuning);
        // Every chunk starts resident and empty.
        for cy in 0..grid.chunks.rows {
            for cx in 0..grid.chunks.cols {
                grid.make_resident(cx, cy);
            }
        }
        grid
    }

    /// A world of the tuning's size with nothing resident: every chunk is
    /// empty air, or packed in the store once something puts it there,
    /// until the live window comes near.
    fn unloaded(tuning: &Tuning) -> Self {
        let width = tuning.arena_columns * tuning.pixels_per_tile;
        let height = tuning.arena_rows * tuning.pixels_per_tile;
        Self {
            weather: false,
            chunks: chunks::ChunkMap::new(width, height, false),
            active: activity::Activity::default(),
            sleep: sleep::SimChunks::new(width, height),
            quiet_moves: false,
            diffusing: false,
            render_cache: render::RenderCache::new(width, height),
            view: None,
            snow_view: None,
            blast_box: None,
            blast_budget: 0,
            visit_stamp: 0,
            motes: Vec::new(),
            grains: Vec::new(),
            parked: Default::default(),
            parked_count: 0,
            bodies: Vec::new(),
            support_dirty: Vec::new(),
            soundscape: Default::default(),
            width,
            height,
            pixel_size: tuning.pixel_size(),
            tick: 0,
            dirty: true,
            ..Default::default()
        }
    }

    /// Give chunk (`cx`, `cy`) a slot, cleared to empty air at ambient
    /// temperature. Returns the slot.
    fn make_resident(&mut self, cx: i32, cy: i32) -> usize {
        let slot = self.chunks.assign(cx, cy);
        if self.chunks.capacity() * chunks::CHUNK_CELLS > self.cells.len() {
            self.fit_to_capacity();
        }
        let range = chunks::ChunkMap::slot_start(slot)..chunks::ChunkMap::slot_start(slot + 1);
        self.cells[range.clone()].fill(None);
        self.flow[range.clone()].fill(Vec2::ZERO);
        self.phase[range.clone()].fill(Vec2::ZERO);
        self.base_colors[range.clone()].fill([0; 4]);
        self.carried[range.clone()].fill(false);
        self.stain[range.clone()].fill(0);
        self.temperature[range.clone()].fill(20.0);
        self.thermal_delta[range.clone()].fill(0.0);
        self.damage[range.clone()].fill(0.0);
        self.life[range.clone()].fill(0);
        self.updated[range.clone()].fill(0);
        self.pressure[range.clone()].fill(0);
        self.visited[range.clone()].fill(0);
        self.airborne[range.clone()].fill(None);
        self.airborne_count[range].fill(0);
        self.active.clear_slot(slot);
        self.sleep.clear_slot(slot, self.tick);
        self.render_cache.clear_slot(&self.chunks, slot);
        slot
    }

    /// The low 32 bits of the tick, as `updated` keeps it.
    fn stamp(&self) -> u32 {
        self.tick as u32
    }

    /// Deterministic per-cell dice for this tick. Reset scenes stay repeatable,
    /// while neighboring cells and consecutive ticks still decorrelate.
    fn roll(&self, pos: GridPosition, salt: u32) -> u32 {
        let tick = (self.tick as u32).wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
        crate::world::materials::noise(pos.x ^ salt as i32, pos.y ^ tick as i32)
    }

    /// Index of the cell in the per-cell arrays, if it is in the world and
    /// its chunk is resident.
    #[inline(always)]
    pub fn index(&self, pos: GridPosition) -> Option<usize> {
        self.chunks.index(pos.x, pos.y)
    }

    /// The cell at index `i` of the per-cell arrays.
    #[inline(always)]
    pub fn cell_of(&self, i: usize) -> GridPosition {
        self.chunks.cell_of(i)
    }

    /// Index of the cell (`dx`, `dy`) away from `cell` (whose index is `i`),
    /// by arithmetic inside its chunk and by looking the chunk up beyond it.
    #[inline(always)]
    fn step_index(&self, i: usize, cell: GridPosition, dx: i32, dy: i32) -> Option<usize> {
        let (lx, ly) = (
            (cell.x & (chunks::CHUNK - 1)) + dx,
            (cell.y & (chunks::CHUNK - 1)) + dy,
        );
        if (0..chunks::CHUNK).contains(&lx) && (0..chunks::CHUNK).contains(&ly) {
            Some((i as isize + (dy * chunks::CHUNK + dx) as isize) as usize)
        } else {
            self.chunks.index(cell.x + dx, cell.y + dy)
        }
    }

    #[inline(always)]
    pub fn get(&self, pos: GridPosition) -> Option<TerrainKind> {
        self.index(pos).and_then(|index| self.cells[index])
    }

    pub fn put(&mut self, pos: GridPosition, kind: Option<TerrainKind>) {
        if let Some(index) = self.index(pos) {
            let occludes = render::occludes;
            if self.cells[index].is_some_and(occludes) != kind.is_some_and(occludes) {
                self.render_cache
                    .note_occluder(&self.chunks, pos, kind.is_some_and(occludes));
            }
            if self.cells[index].is_some_and(TerrainKind::is_solid)
                && kind.is_none_or(|kind| !kind.is_solid() || kind.is_powder())
            {
                self.support_dirty.extend(Self::neighbors(pos));
            }
            if self.cells[index].is_some_and(TerrainKind::is_solid)
                || kind.is_some_and(TerrainKind::is_solid)
            {
                self.render_cache.mark(&self.chunks, pos);
            }
            self.cells[index] = kind;
            self.base_colors[index] = kind.map_or([0; 4], |kind| kind.color(pos.x, pos.y));
            self.carried[index] = false;
            self.temperature[index] = match kind {
                Some(TerrainKind::Fire) => 720.0,
                Some(TerrainKind::Steam) => 110.0,
                Some(TerrainKind::Lava) => 1000.0,
                _ => 20.0,
            };
            self.life[index] = kind.map_or(0, TerrainKind::lifetime);
            if kind.is_some_and(TerrainKind::is_gas) {
                let variation = crate::world::materials::noise(pos.x + self.tick as i32, pos.y);
                self.life[index] =
                    (self.life[index] as f32 * (0.5 + (variation % 100) as f32 * 0.01)) as u16;
                self.flow[index] = Vec2::new((variation % 31) as f32 * 0.012 - 0.18, 0.35);
                self.phase[index] = Vec2::new(0.0, (variation % 100) as f32 * 0.01);
            } else {
                self.flow[index] = Vec2::ZERO;
                self.phase[index] = Vec2::ZERO;
            }
            self.stain[index] = 0;
            self.refresh_activity(index);
            self.refresh_reaction_neighbors(pos);
            self.damage[index] = 0.0;
            self.updated[index] = self.stamp();
            self.dirty = true;
            self.wake(pos);
        }
    }

    /// Put `kind` at `pos` in a color of its own (a creature turned to
    /// stone keeps the look it had).
    pub fn put_colored(&mut self, pos: GridPosition, kind: TerrainKind, color: [u8; 4]) {
        self.put(pos, Some(kind));
        if let Some(index) = self.index(pos) {
            self.base_colors[index] = color;
            self.carried[index] = true;
        }
    }

    /// World units per cell.
    pub fn cell_size(&self) -> f32 {
        self.pixel_size
    }

    /// Arena size in cells.
    pub fn size_in_cells(&self) -> Vec2 {
        Vec2::new(self.width as f32, self.height as f32)
    }

    pub fn world_to_cell(&self, point: Vec2) -> GridPosition {
        GridPosition {
            x: (point.x / self.pixel_size + self.width as f32 * 0.5).floor() as i32,
            y: (point.y / self.pixel_size + self.height as f32 * 0.5).floor() as i32,
        }
    }

    pub fn cell_to_world(&self, pos: GridPosition) -> Vec2 {
        Vec2::new(
            (pos.x as f32 + 0.5 - self.width as f32 * 0.5) * self.pixel_size,
            (pos.y as f32 + 0.5 - self.height as f32 * 0.5) * self.pixel_size,
        )
    }
}

impl LevelGrid {
    fn props_blasted(&mut self, _position: Vec2, _radius: f32, _impulse: f32) {}
    fn flash(&mut self, _position: Vec2, _color: Vec3, _radius: f32, _lifetime: u16) {}

    /// One fixed tick of every material system, in dependency order.
    pub fn step_materials(&mut self, dt: f32, gravity: f32, player: Option<Vec2>) {
        use crate::timed;
        timed!("sim powders", self.step_powders(player));
        timed!("sim fluids", self.step_fluids(player));
        timed!("sim chemistry", self.step_chemistry());
        timed!("sim reactions", self.step_reactions());
        timed!("sim detached", self.settle_detached());
        timed!("sim grains", self.step_grains(dt, gravity, player));
        timed!("sim motes", self.step_motes(dt, gravity));
        if self.weather {
            self.step_weather(player);
        }
    }
}
