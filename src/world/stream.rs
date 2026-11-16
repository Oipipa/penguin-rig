//! Keeping the chunks near the player resident and the rest packed in the
//! store (`store.rs`). The live window decides: chunks in it (and one
//! around it) are unpacked at once when they are needed, those a little
//! further out are unpacked ahead of need a few per tick, and those well
//! behind are packed away a few per tick. Resident memory therefore stays
//! the size of the live window, however large the world.
use super::chunks::{CHUNK, CHUNK_CELLS, ChunkMap};
use super::sleep::ChunkRect;
use super::store::{Decoded, StoredChunk};
use super::*;

/// Chunks beyond the live window unpacked ahead of need.
const RESIDENT_PAD: i32 = 2;
/// Chunks beyond the live window before a resident chunk is packed away: a
/// wider margin than `RESIDENT_PAD`, so walking back and forth across a
/// chunk border does not pack and unpack the same chunks every time.
const EVICT_PAD: i32 = 3;
/// Chunks unpacked ahead of need per tick, nearest first. (What the camera
/// shows is unpacked at once when it is painted; see `bring_in`.)
const PRELOAD_BUDGET: usize = 8;
/// Chunks packed away per tick.
const STOW_BUDGET: usize = 32;
/// Free slots below the resident ones tolerated before they are packed
/// down, and how many chunks are moved down per tick while packing.
const COMPACT_SLACK: usize = 32;
const COMPACT_BUDGET: usize = 24;

impl ChunkRect {
    /// Grown by `pad` chunks on every side and clipped to `cols` x `rows`.
    pub(super) fn padded(self, pad: i32, cols: i32, rows: i32) -> ChunkRect {
        ChunkRect {
            cx0: (self.cx0 - pad).max(0),
            cy0: (self.cy0 - pad).max(0),
            cx1: (self.cx1 + pad).min(cols - 1),
            cy1: (self.cy1 + pad).min(rows - 1),
        }
    }
}

impl LevelGrid {
    /// Bring the resident set in line with the live window: unpack what is
    /// missing in and around it, nearest first, a few chunks a tick, and
    /// pack away what is far behind.
    pub(super) fn stream(&mut self) {
        let (cols, rows) = (self.chunks.cols, self.chunks.rows);
        let live = self.sleep.live();
        if live.cx0 > live.cx1 || live.cy0 > live.cy1 {
            return;
        }
        if self.store.is_empty() && self.chunks.resident_count() == (cols * rows) as usize {
            // Everything is resident and nothing was ever packed.
            if live == self.sleep.all() {
                return;
            }
        }
        let ahead = live.padded(RESIDENT_PAD, cols, rows);
        let keep = live.padded(EVICT_PAD, cols, rows);
        let kept = ((keep.cx1 - keep.cx0 + 1) * (keep.cy1 - keep.cy0 + 1)) as usize;
        let (w, h) = self.live_span;
        let room = (((w + 2 * EVICT_PAD) * (h + 2 * EVICT_PAD)).max(0) as usize).max(kept);
        let mut missing: Vec<(i32, i32)> = crate::timed!(
            "  stream look",
            (ahead.cy0..=ahead.cy1)
                .flat_map(|cy| (ahead.cx0..=ahead.cx1).map(move |cx| (cx, cy)))
                .filter(|&(cx, cy)| self.chunks.slot(cx, cy).is_none())
                .collect()
        );
        // Pack away what is far behind, a few chunks a tick; but when the
        // window has jumped (a teleport), all it left behind that would not
        // leave room for where it is now, at once, before anything comes in.
        let resident = self.chunks.resident_count();
        let inside = (keep.cy0..=keep.cy1)
            .flat_map(|cy| (keep.cx0..=keep.cx1).map(move |cx| (cx, cy)))
            .filter(|&(cx, cy)| self.chunks.slot(cx, cy).is_some())
            .count();
        if resident > inside {
            let budget = STOW_BUDGET.max((resident + missing.len()).saturating_sub(room));
            // From the highest slots down, so the top of the arrays empties.
            let far: Vec<(i32, i32)> = self
                .chunks
                .resident()
                .rev()
                .filter(|&(_, cx, cy)| !keep.contains(cx, cy))
                .map(|(_, cx, cy)| (cx, cy))
                .take(budget)
                .collect();
            crate::timed!("  stream stow", {
                for (cx, cy) in far {
                    self.stow(cx, cy);
                }
            });
        }
        if !missing.is_empty() {
            let (mid_x, mid_y) = (
                (live.cx0 + live.cx1) as f32 * 0.5,
                (live.cy0 + live.cy1) as f32 * 0.5,
            );
            missing.sort_unstable_by(|a, b| {
                let d = |(cx, cy): (i32, i32)| {
                    (cx as f32 - mid_x).powi(2) + (cy as f32 - mid_y).powi(2)
                };
                d(*a).total_cmp(&d(*b))
            });
            missing.truncate(PRELOAD_BUDGET);
            crate::timed!("  stream unstow", self.unstow_many(&missing));
        }
        // What stays resident is packed into the lowest slots, a few a tick,
        // and the arrays keep room for about as much as the window keeps.
        if self.chunks.capacity() > self.chunks.resident_count() + COMPACT_SLACK {
            crate::timed!("  stream compact", self.compact_slots(COMPACT_BUDGET));
        }
        self.keep_room(room + room / 8);
    }

    /// Unpack at once every chunk overlapping the cells of `rect` that is
    /// packed away: what the camera is about to show, or where the player
    /// is about to appear.
    pub fn bring_in(&mut self, rect: render::CellRect) {
        let area = ChunkRect::covering(rect, self.chunks.cols, self.chunks.rows);
        let missing: Vec<(i32, i32)> = (area.cy0..=area.cy1)
            .flat_map(|cy| (area.cx0..=area.cx1).map(move |cx| (cx, cy)))
            .filter(|&(cx, cy)| self.chunks.slot(cx, cy).is_none())
            .collect();
        self.unstow_many(&missing);
    }

    /// Unpack at once every packed chunk a box of world space touches.
    pub fn bring_in_box(&mut self, center: Vec2, half: Vec2) {
        let (min, max) = (
            self.world_to_cell(center - half),
            self.world_to_cell(center + half),
        );
        self.bring_in(render::CellRect {
            x0: min.x - 1,
            y0: min.y - 1,
            x1: max.x + 1,
            y1: max.y + 1,
        });
    }

    /// Unpack several chunks, decoding them side by side on the task pool
    /// (their shades are the costly part), then putting them in place.
    pub(super) fn unstow_many(&mut self, chunks: &[(i32, i32)]) {
        if chunks.is_empty() {
            return;
        }
        let packed: Vec<Option<StoredChunk>> = chunks
            .iter()
            .map(|&(cx, cy)| self.store.remove(&self.chunks.chunk_number(cx, cy)))
            .collect();
        let decoded: Vec<Option<Decoded>> = crate::timed!(
            "  unstow decode",
            if chunks.len() < 2 {
                chunks
                    .iter()
                    .zip(&packed)
                    .map(|(&(cx, cy), packed)| packed.as_ref().map(|p| p.decode(cx, cy)))
                    .collect()
            } else {
                let pool =
                    bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default);
                let mut decoded = pool.scope(|scope| {
                    for (n, (&(cx, cy), packed)) in chunks.iter().zip(&packed).enumerate() {
                        scope.spawn(async move { (n, packed.as_ref().map(|p| p.decode(cx, cy))) });
                    }
                });
                decoded.sort_unstable_by_key(|(n, _)| *n);
                decoded.into_iter().map(|(_, d)| d).collect()
            }
        );
        for ((&(cx, cy), packed), decoded) in chunks.iter().zip(&packed).zip(&decoded) {
            #[cfg(test)]
            crate::dev::profile::add("  unstowed (count)", std::time::Duration::from_micros(1));
            let slot = crate::timed!("  unstow make resident", self.make_resident(cx, cy));
            if let (Some(packed), Some(decoded)) = (packed, decoded) {
                crate::timed!("  unstow install", self.install(packed, decoded, slot));
            }
            crate::timed!("  unstow settle", self.settle_unpacked(slot, cx, cy));
        }
    }

    /// Pack chunk (`cx`, `cy`) into the store and free its slot.
    pub(super) fn stow(&mut self, cx: i32, cy: i32) {
        #[cfg(test)]
        crate::dev::profile::add("  stowed (count)", std::time::Duration::from_micros(1));
        let Some(packed) = self.pack(cx, cy) else {
            return;
        };
        let slot = self.chunks.release(cx, cy).unwrap();
        self.active.clear_slot(slot);
        // A free slot holds nothing, so nothing counts it by mistake.
        let start = ChunkMap::slot_start(slot);
        self.cells[start..start + CHUNK_CELLS].fill(None);
        let chunk = self.chunks.chunk_number(cx, cy);
        self.store.insert(chunk, packed);
    }

    /// Rebuild what is derived from a freshly unpacked chunk's cells: the
    /// activity lists, liquid counts, reaction fronts along its edges, what
    /// blocks light in it, and the cells its parked grains hold.
    fn settle_unpacked(&mut self, slot: usize, cx: i32, cy: i32) {
        let start = ChunkMap::slot_start(slot);
        for i in start..start + CHUNK_CELLS {
            if self.cells[i].is_some() || self.stain[i] > 0 {
                self.refresh_activity(i);
            }
        }
        // Reactive cells just across its edges may face something new.
        let (x0, y0) = (cx * CHUNK - 1, cy * CHUNK - 1);
        let (x1, y1) = (x0 + CHUNK + 1, y0 + CHUNK + 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if (x == x0 || x == x1 || y == y0 || y == y1)
                    && let Some(j) = self.chunks.index(x, y)
                    && self.active.reactive.contains(j)
                {
                    self.refresh_reaction_front(j);
                }
            }
        }
        let chunk = self.chunks.chunk_number(cx, cy);
        let held: Vec<(Vec2, TerrainKind)> = self
            .parked
            .get(&chunk)
            .map(|grains| grains.iter().map(|g| (g.position, g.kind)).collect())
            .unwrap_or_default();
        for (position, kind) in held {
            self.reserve_grain(position, Some(kind));
        }
        self.render_cache.clear_slot(&self.chunks, slot);
        if self.sleep.live().contains(cx, cy) {
            self.unpark(cx, cy);
        }
    }

    /// The topmost solid, non-powder row of column `x` (-1 if none), read
    /// from resident cells or, for stored chunks, from their packed tops.
    pub(super) fn column_top(&self, x: i32) -> i32 {
        if x < 0 || x >= self.width {
            return -1;
        }
        let cx = x / CHUNK;
        for cy in (0..self.chunks.rows).rev() {
            if let Some(slot) = self.chunks.slot(cx, cy) {
                let start = ChunkMap::slot_start(slot) + (x % CHUNK) as usize;
                for ly in (0..CHUNK).rev() {
                    let solid = self.cells[start + (ly * CHUNK) as usize]
                        .is_some_and(|kind| kind.is_solid() && !kind.is_powder());
                    if solid && cy * CHUNK + ly < self.height {
                        return cy * CHUNK + ly;
                    }
                }
            } else if let Some(top) = self
                .store
                .get(&self.chunks.chunk_number(cx, cy))
                .and_then(|stored| stored.top(x % CHUNK))
            {
                return cy * CHUNK + top;
            }
        }
        -1
    }

    /// Put a packed chunk straight into the store at (`cx`, `cy`), for
    /// building a world larger than memory from pieces generated one at a
    /// time. The chunk must not be resident.
    #[cfg(test)]
    pub(super) fn store_chunk(&mut self, cx: i32, cy: i32, packed: StoredChunk) {
        debug_assert!(self.chunks.slot(cx, cy).is_none());
        let chunk = self.chunks.chunk_number(cx, cy);
        self.store.insert(chunk, packed);
    }

    /// The material of a cell wherever it is, resident or packed away
    /// (unpacking nothing, but slow for a packed one): for the opening
    /// snowfall, which falls over sky not yet brought in.
    pub fn material_anywhere(&self, cell: GridPosition) -> Option<TerrainKind> {
        if let Some(i) = self.index(cell) {
            return self.cells[i];
        }
        if cell.x < 0 || cell.y < 0 || cell.x >= self.width || cell.y >= self.height {
            return None;
        }
        let (cx, cy) = ChunkMap::chunk_of(cell.x, cell.y);
        let stored = self.store.get(&self.chunks.chunk_number(cx, cy))?;
        let local = ((cell.y - cy * CHUNK) * CHUNK + cell.x - cx * CHUNK) as usize;
        stored.kinds().nth(local).flatten()
    }

    /// The material and temperature of a cell wherever it is, resident or
    /// packed away: for tests that look at the far side of the world.
    #[cfg(test)]
    pub fn peek(&self, cell: GridPosition) -> (Option<TerrainKind>, f32) {
        if let Some(i) = self.index(cell) {
            return (self.cells[i], self.temperature[i]);
        }
        if cell.x < 0 || cell.y < 0 || cell.x >= self.width || cell.y >= self.height {
            return (None, 20.0);
        }
        let (cx, cy) = ChunkMap::chunk_of(cell.x, cell.y);
        let Some(stored) = self.store.get(&self.chunks.chunk_number(cx, cy)) else {
            return (None, 20.0);
        };
        let local = ((cell.y - cy * CHUNK) * CHUNK + cell.x - cx * CHUNK) as usize;
        (
            stored.kinds().nth(local).flatten(),
            stored.temperature(local),
        )
    }

    /// Whether a box overlaps solid material, looking into packed chunks too.
    #[cfg(test)]
    pub fn overlaps_box_anywhere(&self, center: Vec2, half: Vec2) -> bool {
        let min = self.world_to_cell(center - half + Vec2::splat(0.001));
        let max = self.world_to_cell(center + half - Vec2::splat(0.001));
        (min.y..=max.y).any(|y| {
            (min.x..=max.x).any(|x| {
                let cell = GridPosition { x, y };
                x < 0
                    || x >= self.width
                    || y < 0
                    || self.peek(cell).0.is_some_and(TerrainKind::is_solid)
            })
        })
    }

    /// Bytes of packed chunks in the store.
    #[cfg(test)]
    pub fn stored_bytes(&self) -> usize {
        self.store.values().map(StoredChunk::bytes).sum()
    }
}
