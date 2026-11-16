//! Minimal simulation-side view helpers; the editor owns presentation.

use super::{GridPosition, TerrainKind, chunks::ChunkMap};

pub const VIEW_MARGIN: i32 = 48;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl CellRect {
    pub const EMPTY: Self = Self {
        x0: 0,
        y0: 0,
        x1: -1,
        y1: -1,
    };
    pub fn contains(self, p: GridPosition) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }
    pub fn is_empty(self) -> bool {
        self.x0 > self.x1 || self.y0 > self.y1
    }
    pub fn width(self) -> i32 {
        (self.x1 - self.x0 + 1).max(0)
    }
    pub fn height(self) -> i32 {
        (self.y1 - self.y0 + 1).max(0)
    }
    pub fn intersect(self, b: Self) -> Self {
        Self {
            x0: self.x0.max(b.x0),
            y0: self.y0.max(b.y0),
            x1: self.x1.min(b.x1),
            y1: self.y1.min(b.y1),
        }
    }
}

#[derive(Default, Clone)]
pub struct RenderCache;
impl RenderCache {
    pub fn new(_width: i32, _height: i32) -> Self {
        Self
    }
    pub fn clear_slot(&mut self, _map: &ChunkMap, _slot: usize) {}
    pub fn fit(&mut self, _slots: usize) {}
    pub fn relocate(&mut self, _from: usize, _to: usize) {}
    pub fn keep_room(&mut self, _slots: usize) {}
    pub fn mark(&mut self, _map: &ChunkMap, _cell: GridPosition) {}
    pub fn mark_color(&mut self, _map: &ChunkMap, _cell: GridPosition) {}
    pub fn note_occluder(&mut self, _map: &ChunkMap, _cell: GridPosition, _value: bool) {}
    pub fn sky_share(&self, _map: &ChunkMap, _cell: GridPosition) -> f32 {
        0.5
    }
}

pub fn occludes(kind: TerrainKind) -> bool {
    kind.is_solid() && !kind.is_powder()
}

pub fn stained(color: [u8; 4], stain: u8) -> [u8; 4] {
    if stain == 0 {
        return color;
    }
    let t = stain as f32 / 255.0;
    let mut out = color;
    for c in 0..3 {
        out[c] = (color[c] as f32 * (1.0 - t) + super::materials::BLOOD_STAIN[c] * t) as u8;
    }
    out
}
