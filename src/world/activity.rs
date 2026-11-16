//! Dense per-chunk sets of the cells that need visiting: powders, liquids,
//! gases, reactive cells and cells exchanging heat, so each pass costs what
//! is active, never the size of the world.

use super::chunks::{CHUNK_CELLS, ChunkMap};
use super::sleep::ChunkRect;
use super::*;

/// Within this many degrees of ambient (20) a cell counts as cold: it leaves
/// the heat simulation and snaps to 20. Heat spread thinner than this does
/// nothing visible, and tracking it let one blast warm half the map.
pub const HEAT_EPSILON: f32 = 0.5;

/// Cells of one kind of activity (hot, gas, reacting), kept per chunk slot
/// (see `chunks.rs`) so a pass can visit just the ones in a region: the live
/// window around the player, or the painted view. Membership changes as
/// material moves in constant time, and visiting a region costs what is in
/// it, never what is in the rest of the world.
#[derive(Default)]
#[cfg_attr(test, derive(Clone))]
pub struct ChunkSet {
    /// Cell indices in each slot's chunk, in no particular order.
    buckets: Vec<Vec<u32>>,
    /// Each cell's place in its chunk's bucket, or `ABSENT`. A chunk holds at
    /// most `CHUNK_CELLS` cells, so two bytes suffice.
    places: Vec<u16>,
    /// Material occupancy in each row of a chunk. The awake material passes
    /// can jump directly to the next occupied cell in reading order.
    rows: Vec<[u32; chunks::CHUNK as usize]>,
    len: usize,
}

const ABSENT: u16 = u16::MAX;

impl ChunkSet {
    /// Keep room for `slots` chunk slots (see `keep_room`).
    pub fn keep_room(&mut self, slots: usize) {
        keep_room(&mut self.places, slots);
        if self.rows.capacity() < slots {
            self.rows.reserve_exact(slots - self.rows.len());
        } else if self.rows.capacity() > slots * 2 && self.rows.len() <= slots {
            self.rows.shrink_to(slots);
        }
        if self.buckets.capacity() > slots * 2 && self.buckets.len() <= slots {
            self.buckets.shrink_to(slots);
        }
    }

    /// Hold `slots` chunk slots (those dropped must be empty).
    pub fn fit(&mut self, slots: usize) {
        debug_assert!(self.buckets.iter().skip(slots).all(Vec::is_empty));
        self.buckets.resize_with(slots, Vec::new);
        fit_cells(&mut self.places, slots, ABSENT);
        self.rows.resize(slots, [0; chunks::CHUNK as usize]);
    }

    /// The chunk in slot `from` moved to the free slot `to`.
    pub fn relocate(&mut self, from: usize, to: usize) {
        debug_assert!(self.buckets[to].is_empty());
        let mut bucket = std::mem::take(&mut self.buckets[from]);
        for i in &mut bucket {
            *i = (to * CHUNK_CELLS) as u32 | (*i & (CHUNK_CELLS as u32 - 1));
        }
        self.buckets[to] = bucket;
        self.rows[to] = self.rows[from];
        self.rows[from] = [0; chunks::CHUNK as usize];
        move_cells(&mut self.places, from, to);
        let old = ChunkMap::slot_start(from);
        self.places[old..old + CHUNK_CELLS].fill(ABSENT);
    }

    /// Forget every member in `slot` (its chunk left, or a new one came).
    pub fn clear_slot(&mut self, slot: usize) {
        let bucket = std::mem::take(&mut self.buckets[slot]);
        self.len -= bucket.len();
        for i in bucket {
            self.places[i as usize] = ABSENT;
        }
        self.rows[slot] = [0; chunks::CHUNK as usize];
    }

    pub fn contains(&self, index: usize) -> bool {
        self.places.get(index).is_some_and(|&place| place != ABSENT)
    }

    pub fn set(&mut self, index: usize, present: bool) {
        let place = self.places[index];
        let slot = index / CHUNK_CELLS;
        let local = index % CHUNK_CELLS;
        let row = local / chunks::CHUNK as usize;
        let bit = 1_u32 << (local % chunks::CHUNK as usize);
        if present && place == ABSENT {
            let bucket = &mut self.buckets[slot];
            self.places[index] = bucket.len() as u16;
            bucket.push(index as u32);
            self.rows[slot][row] |= bit;
            self.len += 1;
        } else if !present && place != ABSENT {
            let bucket = &mut self.buckets[slot];
            bucket.swap_remove(place as usize);
            if let Some(&moved) = bucket.get(place as usize) {
                self.places[moved as usize] = place;
            }
            self.places[index] = ABSENT;
            self.rows[slot][row] &= !bit;
            self.len -= 1;
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(super) fn row_mask(&self, slot: usize, row: usize) -> u32 {
        self.rows[slot][row]
    }

    pub(super) fn has_in_slot(&self, slot: usize) -> bool {
        !self.buckets[slot].is_empty()
    }

    pub(super) fn extend_slot_ordered(&self, slot: usize, out: &mut Vec<usize>) {
        let start = slot * CHUNK_CELLS;
        for (row, &bits) in self.rows[slot].iter().enumerate() {
            let mut bits = bits;
            while bits != 0 {
                let x = bits.trailing_zeros() as usize;
                out.push(start + row * chunks::CHUNK as usize + x);
                bits &= bits - 1;
            }
        }
    }

    /// Every member, slot by slot.
    #[cfg(test)]
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.buckets
            .iter()
            .flat_map(|bucket| bucket.iter().map(|&i| i as usize))
    }

    /// Append the members in the resident chunks of `area` to `out`, chunk
    /// row by chunk row.
    pub fn gather(&self, map: &ChunkMap, area: ChunkRect, out: &mut Vec<usize>) {
        if self.len == 0 {
            return;
        }
        for cy in area.cy0..=area.cy1 {
            for cx in area.cx0..=area.cx1 {
                if let Some(slot) = map.slot(cx, cy) {
                    out.extend(self.buckets[slot].iter().map(|&i| i as usize));
                }
            }
        }
    }
}

#[derive(Default)]
#[cfg_attr(test, derive(Clone))]
pub struct Activity {
    pub powders: ChunkSet,
    pub liquids: ChunkSet,
    pub gases: ChunkSet,
    /// Cells with chemistry of their own every tick, hot or not: acid eating
    /// and diluting, blood staining (see `reactions.rs`).
    pub reactive: ChunkSet,
    /// Reactive cells with at least one neighboring material they can affect.
    pub reaction_front: ChunkSet,
    /// Reused snapshot because reactions mutate the frontier while it runs.
    pub reaction_work: Vec<usize>,
    pub hot: ChunkSet,
    /// Reused heat frontier; a hot lava bank can hold thousands of cells.
    pub heat_work: Vec<usize>,
    pub heat_seen: Vec<bool>,
    pub pressure_work: PressureWork,
}

#[derive(Default)]
#[cfg_attr(test, derive(Clone))]
pub struct PressureWork {
    pub liquids: Vec<usize>,
    pub region: Vec<usize>,
    pub sources: Vec<GridPosition>,
    pub targets: Vec<GridPosition>,
}

/// Make a per-cell array hold `slots` chunk slots, new ones set to `fill`.
/// (Room for them is normally kept ahead; see `keep_room`.)
pub(super) fn fit_cells<T: Clone>(cells: &mut Vec<T>, slots: usize, fill: T) {
    let len = slots * CHUNK_CELLS;
    if len > cells.len() {
        cells.resize(len, fill);
    } else {
        cells.truncate(len);
    }
}

/// Keep room in a per-cell array for `slots` chunk slots: reserved once, so
/// chunks coming in never make it move, and given back when it holds far
/// more than that.
pub(super) fn keep_room<T>(cells: &mut Vec<T>, slots: usize) {
    let room = slots * CHUNK_CELLS;
    if cells.capacity() < room {
        cells.reserve_exact(room - cells.len());
    } else if cells.capacity() > room + room / 2 && cells.len() <= room {
        cells.shrink_to(room);
    }
}

/// Copy a chunk's cells from slot `from` to slot `to` of a per-cell array.
pub(super) fn move_cells<T: Copy>(cells: &mut [T], from: usize, to: usize) {
    let start = ChunkMap::slot_start(from);
    cells.copy_within(start..start + CHUNK_CELLS, ChunkMap::slot_start(to));
}

impl Activity {
    /// Keep room for `slots` chunk slots (see `keep_room`).
    pub fn keep_room(&mut self, slots: usize) {
        for set in self.sets_mut() {
            set.keep_room(slots);
        }
        keep_room(&mut self.heat_seen, slots);
    }

    /// Hold `slots` chunk slots.
    pub fn fit(&mut self, slots: usize) {
        for set in self.sets_mut() {
            set.fit(slots);
        }
        fit_cells(&mut self.heat_seen, slots, false);
    }

    /// The chunk in slot `from` moved to the free slot `to`.
    pub fn relocate(&mut self, from: usize, to: usize) {
        for set in self.sets_mut() {
            set.relocate(from, to);
        }
        move_cells(&mut self.heat_seen, from, to);
    }

    /// Forget everything about the chunk in `slot`.
    pub fn clear_slot(&mut self, slot: usize) {
        for set in self.sets_mut() {
            set.clear_slot(slot);
        }
        let start = ChunkMap::slot_start(slot);
        self.heat_seen[start..start + CHUNK_CELLS].fill(false);
    }

    fn sets_mut(&mut self) -> [&mut ChunkSet; 6] {
        [
            &mut self.powders,
            &mut self.liquids,
            &mut self.gases,
            &mut self.reactive,
            &mut self.reaction_front,
            &mut self.hot,
        ]
    }
}

impl LevelGrid {
    pub(super) fn refresh_activity(&mut self, index: usize) {
        let kind = self.cells[index];
        self.active
            .powders
            .set(index, kind.is_some_and(TerrainKind::is_powder));
        let liquid = kind.is_some_and(TerrainKind::is_liquid);
        if self.active.liquids.contains(index) != liquid {
            self.sleep.note_liquid(index, liquid);
        }
        self.active.liquids.set(index, liquid);
        self.active
            .gases
            .set(index, kind.is_some_and(TerrainKind::is_gas));
        // Stained solids react too: water rinses them.
        self.active.reactive.set(
            index,
            kind.is_some_and(TerrainKind::reacts) || self.stain[index] > 0,
        );
        self.refresh_reaction_front(index);
        self.track_heat(index);
    }

    pub(super) fn refresh_reaction_front(&mut self, index: usize) {
        if !self.active.reactive.contains(index) {
            self.active.reaction_front.set(index, false);
            return;
        }
        let cell = self.cell_of(index);
        let possible = match self.cells[index] {
            Some(kind @ (TerrainKind::Acid | TerrainKind::Lava)) => {
                // Lava reacts at cardinal contacts; diagonal water cannot
                // quench it and should not keep its frontier awake.
                let offsets: &[(i32, i32)] = if kind == TerrainKind::Lava {
                    &[(0, -1), (-1, 0), (1, 0), (0, 1)]
                } else {
                    &[
                        (-1, -1),
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                    ]
                };
                offsets.iter().copied().any(|(dx, dy)| {
                    let near = GridPosition {
                        x: cell.x + dx,
                        y: cell.y + dy,
                    };
                    let Some(j) = self.index(near) else {
                        return false;
                    };
                    let neighbor = self.cells[j];
                    if neighbor == Some(kind) {
                        return false;
                    }
                    kind.reactions()
                        .any(|reaction| reaction.partner.affinity(neighbor) > 0.0)
                })
            }
            Some(TerrainKind::Blood) => [(0, -1), (-1, 0), (1, 0), (0, 1), (-1, -1), (1, -1)]
                .into_iter()
                .any(|(dx, dy)| {
                    self.get(GridPosition {
                        x: cell.x + dx,
                        y: cell.y + dy,
                    })
                    .is_some_and(TerrainKind::is_solid)
                }),
            Some(kind) if kind.is_solid() && self.stain[index] > 0 => Self::neighbors(cell)
                .any(|near| matches!(self.get(near), Some(TerrainKind::Water | TerrainKind::Acid))),
            _ => false,
        };
        self.active.reaction_front.set(index, possible);
    }

    /// A changed material can expose or cover a neighboring reaction site.
    pub(super) fn refresh_reaction_neighbors(&mut self, cell: GridPosition) {
        // Most material movement happens in chunks with no acid, blood,
        // lava or stained rock. Skip nine per-cell lookups in those chunks;
        // at a chunk edge include every slot the 3×3 neighborhood crosses.
        let (cx0, cy0) = ChunkMap::chunk_of(cell.x - 1, cell.y - 1);
        let (cx1, cy1) = ChunkMap::chunk_of(cell.x + 1, cell.y + 1);
        if !(cy0..=cy1).any(|cy| {
            (cx0..=cx1).any(|cx| {
                self.chunks
                    .slot(cx, cy)
                    .is_some_and(|slot| self.active.reactive.has_in_slot(slot))
            })
        }) {
            return;
        }
        for dy in -1..=1 {
            for dx in -1..=1 {
                let near = GridPosition {
                    x: cell.x + dx,
                    y: cell.y + dy,
                };
                if let Some(index) = self.index(near) {
                    if self.active.reactive.contains(index) {
                        self.refresh_reaction_front(index);
                    }
                    // A molten pixel just exposed by a neighboring change
                    // becomes a heat source again. Interior molten pixels
                    // can leave the hot activity set at equilibrium.
                    if dx.abs() + dy.abs() == 1 && self.cells[index] == Some(TerrainKind::Lava) {
                        self.track_heat(index);
                    }
                }
            }
        }
    }

    pub(super) fn track_heat(&mut self, index: usize) {
        let Some(kind) = self.cells[index] else {
            self.active.hot.set(index, false);
            return;
        };
        let temperature = self.temperature[index];
        let enclosed_molten = kind == TerrainKind::Lava
            && temperature == 1000.0
            && Self::neighbors(self.cell_of(index)).all(|near| {
                self.index(near)
                    .is_some_and(|j| self.cells[j] == Some(TerrainKind::Lava))
            });
        self.active.hot.set(
            index,
            !enclosed_molten && (temperature - 20.0).abs() > HEAT_EPSILON,
        );
    }
}
