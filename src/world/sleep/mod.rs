//! Noita-style sleeping chunks. The grid is split into square chunks and the
//! cell-by-cell rules (liquid, powder, pressure) only visit chunks where
//! something changed a moment ago. A settled pool or sand bank costs nothing
//! until a spell, a body, a grain or its own neighbors disturb it; a pool that
//! is still moving keeps exactly the chunks it moves through awake.
//!
//! Anything that changes a cell (`put`, `swap_cells`), moves a grain, or
//! leaves a cell with momentum wakes the chunks around it. Cells that are
//! idle but may still act on their own (a surface pixel free to wander,
//! powder waiting on its dice to sink) keep their chunk awake too, so
//! sleeping never freezes something a full scan would have moved.
use super::chunks::{CHUNK_CELLS, ChunkMap};
use super::*;

/// Cells per side of a simulation chunk.
pub(super) const SIM_CHUNK: i32 = chunks::CHUNK;
#[derive(Clone, Copy)]
pub(super) enum AwakeMaterial {
    Powder,
    Liquid,
}
/// Ticks a chunk stays awake after the last change in or beside it.
const LINGER: u64 = 2;
/// Cells around a change whose chunks wake with it: the rules look at direct
/// and diagonal neighbors, and a falling cell travels up to four cells a tick.
const REACH: i32 = 4;

/// An inclusive rectangle of simulation chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkRect {
    pub cx0: i32,
    pub cy0: i32,
    pub cx1: i32,
    pub cy1: i32,
}

impl ChunkRect {
    pub const EMPTY: ChunkRect = ChunkRect {
        cx0: 0,
        cy0: 0,
        cx1: -1,
        cy1: -1,
    };

    pub fn contains(self, cx: i32, cy: i32) -> bool {
        cx >= self.cx0 && cx <= self.cx1 && cy >= self.cy0 && cy <= self.cy1
    }

    /// Chunks overlapping the cells `rect`, clipped to `cols` x `rows`.
    pub fn covering(rect: render::CellRect, cols: i32, rows: i32) -> ChunkRect {
        let clip = ChunkRect {
            cx0: rect.x0.max(0) / SIM_CHUNK,
            cy0: rect.y0.max(0) / SIM_CHUNK,
            cx1: (rect.x1.min(cols * SIM_CHUNK - 1)) / SIM_CHUNK,
            cy1: (rect.y1.min(rows * SIM_CHUNK - 1)) / SIM_CHUNK,
        };
        if rect.x1 < 0 || rect.y1 < 0 || rect.x0 > rect.x1 || rect.y0 > rect.y1 {
            ChunkRect::EMPTY
        } else {
            clip
        }
    }
}

impl Default for ChunkRect {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Default)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct SimChunks {
    cols: i32,
    rows: i32,
    /// The live window: the only chunks any rule visits this tick (see
    /// `LevelGrid::update_live`). Outside it the world is frozen, as
    /// Noita's is beyond the streamed area, and picks up where it left off
    /// when the player comes back.
    live: ChunkRect,
    /// Per slot (see `chunks.rs`): last tick on which the chunk's cells are
    /// stepped.
    awake: Vec<u64>,
    /// Per slot: last tick on which the chunk seeds the pressure pass. Only
    /// changes to a pool's shape count: a surface pixel wandering along its
    /// own row changes no head of pressure.
    pressure: Vec<u64>,
    /// Per slot: liquid cells in the chunk, so painting can skip dry ones.
    liquid: Vec<u32>,
}

impl SimChunks {
    pub fn new(width: i32, height: i32) -> Self {
        let cols = (width + SIM_CHUNK - 1) / SIM_CHUNK;
        let rows = (height + SIM_CHUNK - 1) / SIM_CHUNK;
        Self {
            cols,
            rows,
            live: ChunkRect {
                cx0: 0,
                cy0: 0,
                cx1: cols - 1,
                cy1: rows - 1,
            },
            awake: Vec::new(),
            pressure: Vec::new(),
            liquid: Vec::new(),
        }
    }

    /// Hold `slots` chunk slots.
    pub fn fit(&mut self, slots: usize) {
        self.awake.resize(slots, 0);
        self.pressure.resize(slots, 0);
        self.liquid.resize(slots, 0);
    }

    /// The chunk in slot `from` moved to the free slot `to`.
    pub fn relocate(&mut self, from: usize, to: usize) {
        self.awake[to] = self.awake[from];
        self.pressure[to] = self.pressure[from];
        self.liquid[to] = self.liquid[from];
    }

    /// A chunk just came into `slot`: nothing liquid counted yet, and awake
    /// for a moment, so whatever it holds gets a look.
    pub fn clear_slot(&mut self, slot: usize, tick: u64) {
        self.awake[slot] = tick + LINGER;
        self.pressure[slot] = tick + LINGER;
        self.liquid[slot] = 0;
    }

    /// A liquid cell appeared at (`added`) or left the cell at index `i`.
    pub fn note_liquid(&mut self, i: usize, added: bool) {
        let Some(count) = self.liquid.get_mut(i / CHUNK_CELLS) else {
            return;
        };
        *count = if added {
            *count + 1
        } else {
            count.saturating_sub(1)
        };
    }

    /// Whether chunk (`cx`, `cy`) may hold any liquid (a chunk that is not
    /// resident is not known to be dry).
    pub fn has_liquid(&self, map: &ChunkMap, cx: i32, cy: i32) -> bool {
        map.slot(cx, cy)
            .is_none_or(|slot| self.liquid.get(slot).is_none_or(|&n| n > 0))
    }

    pub fn rows(&self) -> i32 {
        self.rows
    }

    pub fn cols(&self) -> i32 {
        self.cols
    }

    /// Every chunk.
    pub fn all(&self) -> ChunkRect {
        ChunkRect {
            cx0: 0,
            cy0: 0,
            cx1: self.cols - 1,
            cy1: self.rows - 1,
        }
    }

    pub fn live(&self) -> ChunkRect {
        self.live
    }

    /// Move the live window to `live`. Chunks coming into it wake (and may
    /// re-level their pools), so whatever was frozen mid-move as the player
    /// left moves on again: a chunk's own sleep timer ran out while it was
    /// away, but it never got to settle.
    pub fn set_live(&mut self, map: &ChunkMap, live: ChunkRect, tick: u64) {
        let old = self.live;
        if live == old {
            return;
        }
        self.live = live;
        for cy in live.cy0..=live.cy1 {
            for cx in live.cx0..=live.cx1 {
                if !old.contains(cx, cy)
                    && let Some(slot) = map.slot(cx, cy)
                {
                    self.awake[slot] = self.awake[slot].max(tick + LINGER);
                    self.pressure[slot] = self.pressure[slot].max(tick + LINGER);
                }
            }
        }
    }

    /// Wake every resident chunk overlapping cells `x0..=x1` x `y0..=y1`
    /// until `until`.
    fn wake_box(
        &mut self,
        map: &ChunkMap,
        (x0, y0, x1, y1): (i32, i32, i32, i32),
        until: u64,
        pressure: bool,
    ) {
        if self.cols <= 0 || self.rows <= 0 {
            return;
        }
        let cx0 = (x0.max(0) / SIM_CHUNK).min(self.cols - 1);
        let cx1 = (x1.max(0) / SIM_CHUNK).min(self.cols - 1);
        let cy0 = (y0.max(0) / SIM_CHUNK).min(self.rows - 1);
        let cy1 = (y1.max(0) / SIM_CHUNK).min(self.rows - 1);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let Some(slot) = map.slot(cx, cy) else {
                    continue;
                };
                self.awake[slot] = self.awake[slot].max(until);
                if pressure {
                    self.pressure[slot] = self.pressure[slot].max(until);
                }
            }
        }
    }

    /// Awake, resident chunks of chunk row `cy` at `tick` inside the live
    /// window, left to right: (column, slot).
    pub fn awake_in_row(&self, map: &ChunkMap, cy: i32, tick: u64, out: &mut Vec<(i32, usize)>) {
        out.clear();
        let live = self.live;
        if cy < live.cy0 || cy > live.cy1 {
            return;
        }
        out.extend(
            (live.cx0..=live.cx1)
                .filter_map(|cx| map.slot(cx, cy).map(|slot| (cx, slot)))
                .filter(|&(_, slot)| self.awake[slot] >= tick),
        );
    }

    /// Lower-left cells of the live chunks whose pools changed shape recently.
    pub fn pressure_chunks(&self, map: &ChunkMap, tick: u64) -> Vec<(i32, i32)> {
        let live = self.live;
        let mut out = Vec::new();
        for cy in live.cy0..=live.cy1 {
            for cx in live.cx0..=live.cx1 {
                if map
                    .slot(cx, cy)
                    .is_some_and(|slot| self.pressure[slot] >= tick)
                {
                    out.push((cx * SIM_CHUNK, cy * SIM_CHUNK));
                }
            }
        }
        out
    }

    #[cfg(test)]
    pub fn awake_count(&self, map: &ChunkMap, tick: u64) -> usize {
        let live = self.live;
        (live.cy0..=live.cy1)
            .flat_map(|cy| (live.cx0..=live.cx1).map(move |cx| (cy, cx)))
            .filter_map(|(cy, cx)| map.slot(cx, cy))
            .filter(|&slot| self.awake[slot] >= tick)
            .count()
    }
}

/// Cells simulated beyond the painted view (itself the screen plus
/// `VIEW_MARGIN`). What is just off screen keeps moving, so nothing in sight
/// ever freezes and the edge of the frozen world is never seen.
pub(super) const LIVE_MARGIN: i32 = 96;

impl LevelGrid {
    /// The cells that simulate this tick: the painted view and the default
    /// view around the player, plus `LIVE_MARGIN`. None means all of them
    /// (a grid with no player and no camera, as in most tests).
    fn live_area(&self, player: Option<Vec2>) -> Option<render::CellRect> {
        let around = player.map(|player| {
            // The default window (1280 x 720 at camera scale 0.75), in case
            // the camera has not looked yet or looks elsewhere.
            let half = (Vec2::new(640.0, 360.0) * 0.75 / self.pixel_size).ceil();
            let min = self.world_to_cell(player - half * self.pixel_size);
            let max = self.world_to_cell(player + half * self.pixel_size);
            render::CellRect {
                x0: min.x - render::VIEW_MARGIN,
                y0: min.y - render::VIEW_MARGIN,
                x1: max.x + render::VIEW_MARGIN,
                y1: max.y + render::VIEW_MARGIN,
            }
        });
        let area = match (self.view, around) {
            (None, None) => return None,
            (Some(view), None) => view,
            (None, Some(around)) => around,
            // The camera looks at the player (zoomed out, it may see more
            // than the default window). Right after a teleport it is still
            // panning over from far away: then only the player's
            // surroundings count, or the window would stretch across the
            // whole world in between for a moment and wake all of it.
            (Some(view), Some(around))
                if view.contains(self.world_to_cell(player.unwrap_or_default())) =>
            {
                render::CellRect {
                    x0: view.x0.min(around.x0),
                    y0: view.y0.min(around.y0),
                    x1: view.x1.max(around.x1),
                    y1: view.y1.max(around.y1),
                }
            }
            (Some(_), Some(around)) => around,
        };
        Some(render::CellRect {
            x0: area.x0 - LIVE_MARGIN,
            y0: area.y0 - LIVE_MARGIN,
            x1: area.x1 + LIVE_MARGIN,
            y1: area.y1 + LIVE_MARGIN,
        })
    }

    /// Move the live window with the player and the camera. Grains parked
    /// in chunks it reaches take flight again.
    pub(super) fn update_live(&mut self, player: Option<Vec2>) {
        let area = self.live_area(player);
        if let Some(area) = area {
            // Its size in chunks wherever it lies, even clipped by an edge:
            // what the streaming keeps room for.
            let chunks = |cells: i32| (cells + SIM_CHUNK - 1) / SIM_CHUNK + 1;
            self.live_span = (chunks(area.width()), chunks(area.height()));
        }
        let live = match area {
            None => self.sleep.all(),
            Some(area) => ChunkRect::covering(area, self.sleep.cols(), self.sleep.rows()),
        };
        let old = self.sleep.live();
        if live != old {
            self.sleep.set_live(&self.chunks, live, self.tick);
        }
        // From here on every chunk the window needs is resident.
        crate::timed!("sim stream", self.stream());
        if live == old || self.parked_count == 0 {
            return;
        }
        for cy in live.cy0..=live.cy1 {
            for cx in live.cx0..=live.cx1 {
                if old.contains(cx, cy) {
                    continue;
                }
                if self.chunks.slot(cx, cy).is_some() {
                    self.unpark(cx, cy);
                }
            }
        }
    }

    /// Grains parked in chunk (`cx`, `cy`) take flight again.
    pub(super) fn unpark(&mut self, cx: i32, cy: i32) {
        let chunk = self.chunks.chunk_number(cx, cy);
        if let Some(mut parked) = self.parked.remove(&chunk) {
            self.parked_count -= parked.len();
            self.grains.append(&mut parked);
        }
    }

    /// The simulation chunk a point falls in (clamped to the grid).
    pub(super) fn chunk_of(&self, point: Vec2) -> usize {
        let cell = self.world_to_cell(point);
        let cx = cell.x.clamp(0, self.width - 1) / SIM_CHUNK;
        let cy = cell.y.clamp(0, self.height - 1) / SIM_CHUNK;
        (cy * self.sleep.cols() + cx) as usize
    }

    /// The live window in cells (inclusive; empty when nothing is live).
    pub(super) fn live_cells(&self) -> render::CellRect {
        let live = self.sleep.live();
        render::CellRect {
            x0: live.cx0 * SIM_CHUNK,
            y0: live.cy0 * SIM_CHUNK,
            x1: ((live.cx1 + 1) * SIM_CHUNK).min(self.width) - 1,
            y1: ((live.cy1 + 1) * SIM_CHUNK).min(self.height) - 1,
        }
    }

    /// The live window in cells, corners inclusive (None when nothing is
    /// live), for the creatures' way-finding.
    pub fn live_window(&self) -> Option<(GridPosition, GridPosition)> {
        let live = self.live_cells();
        (live.x1 >= live.x0 && live.y1 >= live.y0).then_some((
            GridPosition {
                x: live.x0,
                y: live.y0,
            },
            GridPosition {
                x: live.x1,
                y: live.y1,
            },
        ))
    }

    /// Whether `cell` is in the live window `live` (from `live_cells`).
    /// Anything above the open top or past an edge counts as the edge cell
    /// under it, so a spark flying out over the top is still live.
    /// A chunk inside the window that is not unpacked yet is not live either.
    pub(super) fn in_live(&self, live: render::CellRect, cell: GridPosition) -> bool {
        let cell = GridPosition {
            x: cell.x.clamp(0, self.width - 1),
            y: cell.y.clamp(0, self.height - 1),
        };
        live.contains(cell) && self.chunks.is_resident(cell.x, cell.y)
    }

    /// Whether the world at `point` is simulating: creatures farther out
    /// rest, and grains and dust there wait or go.
    pub fn is_live(&self, point: Vec2) -> bool {
        self.cells.is_empty() || self.in_live(self.live_cells(), self.world_to_cell(point))
    }

    fn wake_cells(&mut self, min: GridPosition, max: GridPosition, pressure: bool) {
        let until = self.tick + LINGER;
        self.sleep.wake_box(
            &self.chunks,
            (min.x - REACH, min.y - REACH, max.x + REACH, max.y + REACH),
            until,
            pressure,
        );
    }

    /// Something changed at `cell`: step the chunks around it for a couple
    /// more ticks, and let the pressure pass re-level the pool it is in.
    pub(super) fn wake(&mut self, cell: GridPosition) {
        let pressure = !self.quiet_moves;
        self.wake_cells(cell, cell, pressure);
    }

    /// Keep stepping the chunks around `cell` without asking the pressure
    /// pass to look: an idle pixel that may still hop about by itself.
    pub(super) fn keep_awake(&mut self, cell: GridPosition) {
        self.wake_cells(cell, cell, false);
    }

    /// Wake everything overlapping a box of cells.
    pub(super) fn wake_area(&mut self, min: GridPosition, max: GridPosition) {
        self.wake_cells(min, max, true);
    }

    /// Things that move without changing cells still change what cells may
    /// do: the player and swimming bodies hold space that powder and liquid
    /// cannot enter, and release it again as they move on.
    pub(super) fn wake_movers(&mut self, player: Option<Vec2>) {
        if let Some(player) = player {
            let margin = PLAYER_HALF_SIZE + Vec2::splat(self.pixel_size * 2.0);
            let min = self.world_to_cell(player - margin);
            let max = self.world_to_cell(player + margin);
            self.wake_area(min, max);
        }
        for b in 0..self.bodies.len() {
            let body = self.bodies[b];
            let margin = body.half + Vec2::splat(self.pixel_size * 2.0);
            let min = self.world_to_cell(body.start.min(body.end) - margin);
            let max = self.world_to_cell(body.start.max(body.end) + margin);
            self.wake_area(min, max);
        }
    }

    /// Visit every cell of every awake chunk bottom-up, row by row across the
    /// whole grid, left to right when `forward`. Which chunks are awake is
    /// read as each chunk row is reached, so movement from the rows below
    /// wakes the rows above within the same tick, as a full scan would see it.
    pub(super) fn for_awake_cells(
        &mut self,
        forward: bool,
        material: AwakeMaterial,
        mut step: impl FnMut(&mut Self, GridPosition, usize),
    ) {
        let mut chunks = Vec::new();
        let live = self.sleep.live();
        for cy in live.cy0..=live.cy1 {
            self.sleep
                .awake_in_row(&self.chunks, cy, self.tick, &mut chunks);
            if chunks.is_empty() {
                continue;
            }
            if !forward {
                chunks.reverse();
            }
            let y1 = ((cy + 1) * SIM_CHUNK).min(self.height);
            for y in cy * SIM_CHUNK..y1 {
                let local_row = (y - cy * SIM_CHUNK) as usize;
                let row = local_row * SIM_CHUNK as usize;
                for &(cx, slot) in &chunks {
                    let x0 = cx * SIM_CHUNK;
                    let x1 = (x0 + SIM_CHUNK).min(self.width);
                    let start = ChunkMap::slot_start(slot) + row;
                    let width = (x1 - x0) as u32;
                    let valid = ((1_u64 << width) - 1) as u32;
                    let mut cursor = if forward { 0 } else { width };
                    while if forward { cursor < width } else { cursor > 0 } {
                        let mask = match material {
                            AwakeMaterial::Powder => self.active.powders.row_mask(slot, local_row),
                            AwakeMaterial::Liquid => self.active.liquids.row_mask(slot, local_row),
                        } & valid;
                        let remaining = if forward {
                            mask & (u32::MAX << cursor)
                        } else {
                            mask & (u32::MAX >> (32 - cursor))
                        };
                        if remaining == 0 {
                            break;
                        }
                        let local = if forward {
                            remaining.trailing_zeros()
                        } else {
                            31 - remaining.leading_zeros()
                        };
                        let i = start + local as usize;
                        step(
                            self,
                            GridPosition {
                                x: x0 + local as i32,
                                y,
                            },
                            i,
                        );
                        cursor = if forward { local + 1 } else { local };
                    }
                }
            }
        }
    }
}
