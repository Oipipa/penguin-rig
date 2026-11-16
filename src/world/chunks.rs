//! Where each cell's state lives. The world is cut into square chunks; only
//! the chunks near the live window are *resident*, each in a slot of the
//! per-cell arrays (`CHUNK_CELLS` consecutive entries, row by row). The rest
//! are packed into the chunk store (`store.rs`) and unpacked into a free slot
//! when the player comes near. Memory therefore follows what is near the
//! player, not the size of the world.
//!
//! A cell's index is its slot times `CHUNK_CELLS` plus its place in the
//! chunk; `ChunkMap::index` and `ChunkMap::cell_of` convert between the two.
//! Light texels (one per `LIGHT_STEP`² cells) live in the same slots,
//! `TEXELS_PER_SLOT` to a chunk.
use super::GridPosition;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Cells per side of a chunk: the unit the world is simulated, painted,
/// stored and streamed in.
pub const CHUNK: i32 = 32;
const SHIFT: i32 = 5;
const MASK: i32 = CHUNK - 1;
/// Cells in a chunk, and so in a slot of every per-cell array.
pub const CHUNK_CELLS: usize = (CHUNK * CHUNK) as usize;
/// Cells per side of a light texel.
pub const LIGHT_STEP: i32 = 4;
/// Light texels per side of a chunk.
pub const CHUNK_TEXELS: i32 = CHUNK / LIGHT_STEP;
const TEXEL_SHIFT: i32 = 3;
/// Light texels in a chunk, and so in a slot of every per-texel array.
pub const TEXELS_PER_SLOT: usize = (CHUNK_TEXELS * CHUNK_TEXELS) as usize;
const NO_SLOT: u32 = u32::MAX;

/// Which chunks are resident, and in which slot.
#[derive(Default, Clone)]
pub struct ChunkMap {
    /// The world, in cells.
    pub width: i32,
    pub height: i32,
    /// The world, in chunks.
    pub cols: i32,
    pub rows: i32,
    /// Slot of each chunk of the world (`NO_SLOT` while it is stored).
    slot_of: Vec<u32>,
    /// World chunk (x, y) held by each slot, or None while the slot is free.
    held: Vec<Option<(i32, i32)>>,
    /// Free slots, lowest first: chunks coming in fill the bottom, so the
    /// per-cell arrays stay as short as what is resident allows.
    free: BinaryHeap<Reverse<u32>>,
    resident: usize,
}

impl ChunkMap {
    /// A world of `width` x `height` cells, every chunk resident (in slot
    /// order row by row) or none of them.
    pub fn new(width: i32, height: i32, all_resident: bool) -> Self {
        let cols = (width + CHUNK - 1).max(0) / CHUNK;
        let rows = (height + CHUNK - 1).max(0) / CHUNK;
        let chunks = (cols * rows) as usize;
        let mut map = Self {
            width,
            height,
            cols,
            rows,
            slot_of: vec![NO_SLOT; chunks],
            held: Vec::new(),
            free: BinaryHeap::new(),
            resident: 0,
        };
        if all_resident {
            for cy in 0..rows {
                for cx in 0..cols {
                    map.assign(cx, cy);
                }
            }
        }
        map
    }

    /// Slots in use or free: the per-cell arrays hold this many chunks.
    pub fn capacity(&self) -> usize {
        self.held.len()
    }

    /// Chunks resident now.
    pub fn resident_count(&self) -> usize {
        self.resident
    }

    pub fn in_world(&self, cx: i32, cy: i32) -> bool {
        cx >= 0 && cy >= 0 && cx < self.cols && cy < self.rows
    }

    /// The world chunk's number (row by row).
    pub fn chunk_number(&self, cx: i32, cy: i32) -> usize {
        (cy * self.cols + cx) as usize
    }

    /// The slot holding chunk (`cx`, `cy`), if it is resident.
    #[inline(always)]
    pub fn slot(&self, cx: i32, cy: i32) -> Option<usize> {
        if !self.in_world(cx, cy) {
            return None;
        }
        let slot = self.slot_of[self.chunk_number(cx, cy)];
        (slot != NO_SLOT).then_some(slot as usize)
    }

    /// The chunk a slot holds.
    pub fn held(&self, slot: usize) -> Option<(i32, i32)> {
        self.held.get(slot).copied().flatten()
    }

    /// The chunk a cell is in (cells past an edge give chunks past it).
    #[inline]
    pub fn chunk_of(x: i32, y: i32) -> (i32, i32) {
        (x >> SHIFT, y >> SHIFT)
    }

    /// Whether the cell is in the world and its chunk resident.
    #[inline]
    pub fn is_resident(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some()
    }

    /// Whether the cell is in the world but its chunk is stored away.
    #[inline]
    pub fn is_stored(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height && !self.is_resident(x, y)
    }

    /// Index of the cell in the per-cell arrays, if it is in the world and
    /// resident.
    #[inline(always)]
    pub fn index(&self, x: i32, y: i32) -> Option<usize> {
        // Negative coordinates wrap to huge ones: one test per axis.
        if x as u32 >= self.width as u32 || y as u32 >= self.height as u32 {
            return None;
        }
        let slot = self.slot_of[((y >> SHIFT) * self.cols + (x >> SHIFT)) as usize];
        (slot != NO_SLOT).then_some(
            ((slot as usize) << (2 * SHIFT)) | (((y & MASK) << SHIFT) | (x & MASK)) as usize,
        )
    }

    /// The cell at index `i` of the per-cell arrays.
    #[inline(always)]
    pub fn cell_of(&self, i: usize) -> GridPosition {
        let (cx, cy) = self.held[i >> (2 * SHIFT)].unwrap_or((i32::MIN / 2, i32::MIN / 2));
        let local = (i & (CHUNK_CELLS - 1)) as i32;
        GridPosition {
            x: (cx << SHIFT) | (local & MASK),
            y: (cy << SHIFT) | (local >> SHIFT),
        }
    }

    /// The first index of a slot: the cell in its chunk's lower-left corner.
    #[inline]
    pub fn slot_start(slot: usize) -> usize {
        slot << (2 * SHIFT)
    }

    /// Index of light texel (`lx`, `ly`) in the per-texel arrays, if its
    /// chunk is resident.
    #[inline]
    pub fn texel(&self, lx: i32, ly: i32) -> Option<usize> {
        if lx < 0 || ly < 0 {
            return None;
        }
        let slot = self.slot(lx >> TEXEL_SHIFT, ly >> TEXEL_SHIFT)?;
        Some(
            (slot << (2 * TEXEL_SHIFT))
                | (((ly & (CHUNK_TEXELS - 1)) << TEXEL_SHIFT) | (lx & (CHUNK_TEXELS - 1))) as usize,
        )
    }

    /// Put chunk (`cx`, `cy`) in a slot. Returns the slot; when it is a new
    /// one, the per-cell arrays must grow to `capacity()`.
    pub fn assign(&mut self, cx: i32, cy: i32) -> usize {
        let chunk = self.chunk_number(cx, cy);
        debug_assert_eq!(self.slot_of[chunk], NO_SLOT, "chunk already resident");
        let slot = match self.free.pop() {
            Some(Reverse(slot)) => slot as usize,
            None => {
                self.held.push(None);
                self.held.len() - 1
            }
        };
        self.slot_of[chunk] = slot as u32;
        self.held[slot] = Some((cx, cy));
        self.resident += 1;
        slot
    }

    /// Free chunk (`cx`, `cy`)'s slot. Returns the slot it had.
    pub fn release(&mut self, cx: i32, cy: i32) -> Option<usize> {
        let chunk = self.chunk_number(cx, cy);
        let slot = self.slot_of[chunk];
        if slot == NO_SLOT {
            return None;
        }
        self.slot_of[chunk] = NO_SLOT;
        self.held[slot as usize] = None;
        self.free.push(Reverse(slot));
        self.resident -= 1;
        Some(slot as usize)
    }

    /// Every resident chunk: (slot, cx, cy), lowest slot first.
    pub fn resident(&self) -> impl DoubleEndedIterator<Item = (usize, i32, i32)> + '_ {
        self.held
            .iter()
            .enumerate()
            .filter_map(|(slot, held)| held.map(|(cx, cy)| (slot, cx, cy)))
    }

    /// Move up to `budget` chunks from the highest resident slots into the
    /// lowest free ones, then drop the free slots at the top. Returns the
    /// moves, (from, to), for the per-slot arrays to make too; they then
    /// hold `capacity()` slots.
    pub fn compact(&mut self, budget: usize) -> Vec<(usize, usize)> {
        let mut moves = Vec::new();
        loop {
            while let Some(None) = self.held.last() {
                self.held.pop();
            }
            let top = self.held.len();
            // Free slots above the top are gone with it.
            while self
                .free
                .peek()
                .is_some_and(|Reverse(s)| *s as usize >= top)
            {
                self.free.pop();
            }
            if moves.len() >= budget || top == 0 {
                break;
            }
            let Some(&Reverse(to)) = self.free.peek() else {
                break;
            };
            let (from, to) = (top - 1, to as usize);
            if to >= from {
                break;
            }
            self.free.pop();
            let (cx, cy) = self.held[from].take().unwrap();
            self.held[to] = Some((cx, cy));
            let chunk = self.chunk_number(cx, cy);
            self.slot_of[chunk] = to as u32;
            moves.push((from, to));
        }
        // What is left in the heap may include slots now above the top.
        let top = self.held.len() as u32;
        self.free.retain(|Reverse(s)| *s < top);
        moves
    }
}
