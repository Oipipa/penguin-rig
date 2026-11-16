//! The chunk store: chunks away from the live window, packed small. A chunk
//! is kept as the runs of its materials plus only what differs from what
//! its materials and places imply: a shade carried in from elsewhere, heat,
//! fuel spent, a stain, cracks, motion. A chunk of undisturbed rock packs to
//! a few hundred bytes instead of the ~60 KB it takes resident, so the store
//! costs a sliver of what the world would cost in memory.
//!
//! `LevelGrid::stow` packs a resident chunk and frees its slot;
//! `LevelGrid::unstow` unpacks one into a slot and rebuilds what is derived
//! from its cells (activity lists, occluders, grain reservations).
use super::chunks::{CHUNK, CHUNK_CELLS, ChunkMap};
use super::*;

/// One chunk, packed into a single run of bytes (little-endian):
///
/// * seven `u16` counts: runs, colors, heat, life, stain, damage, motion;
/// * `tops`: 32 `i8`, the highest solid, non-powder row of each column (-1 if
///   none), so the sky can find the top of the rock without unpacking;
/// * runs of materials row by row, `[code, length]` (code 0 is empty and
///   `k + 1` is `TerrainKind::ALL[k]`, lengths at most 255);
/// * then only the cells that differ from what their material and place
///   imply, each as a `u16` cell number and its value: a carried color
///   (`[u8; 4]`), heat (`f32`), fuel or lifetime (`u16`), stain (`u8`),
///   damage (`f32`), and motion (flow and phase, four `f32`).
#[derive(Clone, Debug, PartialEq)]
pub struct StoredChunk(Box<[u8]>);

const COUNTS: usize = 7;
const TOPS: usize = COUNTS * 2;
const RUNS: usize = TOPS + CHUNK as usize;

/// A chunk as lists, on its way into or out of its bytes.
#[derive(Default)]
struct Unpacked {
    runs: Vec<[u8; 2]>,
    colors: Vec<(u16, [u8; 4])>,
    heat: Vec<(u16, f32)>,
    life: Vec<(u16, u16)>,
    stain: Vec<(u16, u8)>,
    damage: Vec<(u16, f32)>,
    motion: Vec<(u16, Vec2, Vec2)>,
    tops: [i8; CHUNK as usize],
}

impl Unpacked {
    fn to_bytes(&self) -> StoredChunk {
        let mut out = Vec::with_capacity(
            RUNS + self.runs.len() * 2
                + self.colors.len() * 6
                + self.heat.len() * 6
                + self.life.len() * 4
                + self.stain.len() * 3
                + self.damage.len() * 6
                + self.motion.len() * 18,
        );
        for n in [
            self.runs.len(),
            self.colors.len(),
            self.heat.len(),
            self.life.len(),
            self.stain.len(),
            self.damage.len(),
            self.motion.len(),
        ] {
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
        out.extend(self.tops.iter().map(|&t| t as u8));
        for run in &self.runs {
            out.extend_from_slice(run);
        }
        for (at, color) in &self.colors {
            out.extend_from_slice(&at.to_le_bytes());
            out.extend_from_slice(color);
        }
        for (at, t) in &self.heat {
            out.extend_from_slice(&at.to_le_bytes());
            out.extend_from_slice(&t.to_le_bytes());
        }
        for (at, life) in &self.life {
            out.extend_from_slice(&at.to_le_bytes());
            out.extend_from_slice(&life.to_le_bytes());
        }
        for (at, stain) in &self.stain {
            out.extend_from_slice(&at.to_le_bytes());
            out.push(*stain);
        }
        for (at, damage) in &self.damage {
            out.extend_from_slice(&at.to_le_bytes());
            out.extend_from_slice(&damage.to_le_bytes());
        }
        for (at, flow, phase) in &self.motion {
            out.extend_from_slice(&at.to_le_bytes());
            for v in [flow.x, flow.y, phase.x, phase.y] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        StoredChunk(out.into_boxed_slice())
    }
}

/// Reads a packed chunk front to back.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let out = self.bytes[self.at..self.at + N].try_into().unwrap();
        self.at += N;
        out
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take())
    }
    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take())
    }
}

impl StoredChunk {
    /// Bytes this chunk takes in the store.
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.0.len()
    }

    fn count(&self, n: usize) -> usize {
        u16::from_le_bytes([self.0[2 * n], self.0[2 * n + 1]]) as usize
    }

    /// Materials of the chunk's cells, row by row.
    pub fn kinds(&self) -> impl Iterator<Item = Option<TerrainKind>> + '_ {
        self.0[RUNS..RUNS + self.count(0) * 2]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|run| {
                std::iter::repeat_n(
                    run[0].checked_sub(1).map(|k| TerrainKind::ALL[k as usize]),
                    run[1] as usize,
                )
            })
    }

    /// The temperature of the chunk's `local`th cell.
    #[cfg(test)]
    pub fn temperature(&self, local: usize) -> f32 {
        self.unpacked()
            .heat
            .iter()
            .find(|&&(at, _)| at as usize == local)
            .map_or(20.0, |&(_, t)| t)
    }

    /// The highest solid, non-powder row of column `lx`, if any.
    pub fn top(&self, lx: i32) -> Option<i32> {
        let top = self.0[TOPS + lx as usize] as i8;
        (top >= 0).then_some(top as i32)
    }

    fn unpacked(&self) -> Unpacked {
        let mut read = Reader {
            bytes: &self.0,
            at: RUNS,
        };
        let n: [usize; COUNTS] = std::array::from_fn(|k| self.count(k));
        let mut tops = [-1; CHUNK as usize];
        for (lx, top) in tops.iter_mut().enumerate() {
            *top = self.0[TOPS + lx] as i8;
        }
        Unpacked {
            runs: (0..n[0]).map(|_| read.take()).collect(),
            colors: (0..n[1]).map(|_| (read.u16(), read.take())).collect(),
            heat: (0..n[2]).map(|_| (read.u16(), read.f32())).collect(),
            life: (0..n[3]).map(|_| (read.u16(), read.u16())).collect(),
            stain: (0..n[4])
                .map(|_| (read.u16(), read.take::<1>()[0]))
                .collect(),
            damage: (0..n[5]).map(|_| (read.u16(), read.f32())).collect(),
            motion: (0..n[6])
                .map(|_| {
                    let at = read.u16();
                    let v: [f32; 4] = std::array::from_fn(|_| read.f32());
                    (at, Vec2::new(v[0], v[1]), Vec2::new(v[2], v[3]))
                })
                .collect(),
            tops,
        }
    }
}

fn code(kind: Option<TerrainKind>) -> u8 {
    kind.map_or(0, |kind| kind as u8 + 1)
}

/// A kind's fuel (or lifetime) when freshly placed; gases vary, so theirs is
/// always kept.
fn fresh_life(kind: Option<TerrainKind>) -> Option<u16> {
    match kind {
        Some(kind) if kind.is_gas() => None,
        Some(kind) => Some(kind.lifetime()),
        None => Some(0),
    }
}

impl LevelGrid {
    /// Pack the resident chunk (`cx`, `cy`).
    pub(super) fn pack(&self, cx: i32, cy: i32) -> Option<StoredChunk> {
        let slot = self.chunks.slot(cx, cy)?;
        let start = ChunkMap::slot_start(slot);
        let mut stored = Unpacked {
            tops: [-1; CHUNK as usize],
            ..Default::default()
        };
        for local in 0..CHUNK_CELLS {
            let i = start + local;
            let kind = self.cells[i];
            let c = code(kind);
            match stored.runs.last_mut() {
                Some([last, run]) if *last == c && *run < 255 => *run += 1,
                _ => stored.runs.push([c, 1]),
            }
            let at = local as u16;
            if kind.is_some() && self.carried[i] {
                stored.colors.push((at, self.base_colors[i]));
            }
            if self.temperature[i] != 20.0 {
                stored.heat.push((at, self.temperature[i]));
            }
            if fresh_life(kind) != Some(self.life[i]) {
                stored.life.push((at, self.life[i]));
            }
            if self.stain[i] != 0 {
                stored.stain.push((at, self.stain[i]));
            }
            if self.damage[i] != 0.0 {
                stored.damage.push((at, self.damage[i]));
            }
            if self.flow[i] != Vec2::ZERO || self.phase[i] != Vec2::ZERO {
                stored.motion.push((at, self.flow[i], self.phase[i]));
            }
            if kind.is_some_and(|kind| kind.is_solid() && !kind.is_powder()) {
                let (lx, ly) = (local % CHUNK as usize, (local / CHUNK as usize) as i8);
                stored.tops[lx] = stored.tops[lx].max(ly);
            }
        }
        Some(stored.to_bytes())
    }

    /// Fill the (just assigned, cleared) `slot` from a packed chunk and its
    /// decoding. Only the cells: what is derived from them is rebuilt by
    /// `settle_unpacked`.
    pub(super) fn install(&mut self, packed: &StoredChunk, decoded: &Decoded, slot: usize) {
        let start = ChunkMap::slot_start(slot);
        let stored = packed.unpacked();
        let range = start..start + CHUNK_CELLS;
        self.cells[range.clone()].copy_from_slice(&decoded.kinds[..]);
        self.base_colors[range.clone()].copy_from_slice(&decoded.colors[..]);
        for (life, kind) in self.life[range].iter_mut().zip(decoded.kinds.iter()) {
            *life = kind.map_or(0, TerrainKind::lifetime);
        }
        for &(at, color) in &stored.colors {
            self.base_colors[start + at as usize] = color;
            self.carried[start + at as usize] = true;
        }
        for &(at, t) in &stored.heat {
            self.temperature[start + at as usize] = t;
        }
        for &(at, life) in &stored.life {
            self.life[start + at as usize] = life;
        }
        for &(at, stain) in &stored.stain {
            self.stain[start + at as usize] = stain;
        }
        for &(at, damage) in &stored.damage {
            self.damage[start + at as usize] = damage;
        }
        for &(at, flow, phase) in &stored.motion {
            self.flow[start + at as usize] = flow;
            self.phase[start + at as usize] = phase;
        }
    }

    /// Unpack `packed` into `slot` (decoding it here).
    #[cfg(test)]
    pub(super) fn unpack(&mut self, packed: &StoredChunk, slot: usize, cx: i32, cy: i32) {
        let decoded = packed.decode(cx, cy);
        self.install(packed, &decoded, slot);
    }
}

/// A chunk's materials and their own shades, worked out from its packed
/// form: the costly part of unpacking (every cell's shade is noise), and
/// pure, so many chunks can be decoded at once on other threads.
pub(super) struct Decoded {
    kinds: Box<[Option<TerrainKind>]>,
    colors: Box<[[u8; 4]]>,
}

impl StoredChunk {
    /// Decode the chunk as it lies at (`cx`, `cy`).
    pub fn decode(&self, cx: i32, cy: i32) -> Decoded {
        let (x0, y0) = (cx * CHUNK, cy * CHUNK);
        let mut kinds = vec![None; CHUNK_CELLS].into_boxed_slice();
        let mut colors = vec![[0; 4]; CHUNK_CELLS].into_boxed_slice();
        for (local, kind) in self.kinds().enumerate().take(CHUNK_CELLS) {
            kinds[local] = kind;
            if let Some(kind) = kind {
                let (x, y) = (
                    x0 + (local % CHUNK as usize) as i32,
                    y0 + (local / CHUNK as usize) as i32,
                );
                colors[local] = kind.color(x, y);
            }
        }
        Decoded { kinds, colors }
    }
}
