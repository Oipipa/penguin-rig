//! Dimensions and gravity for a material grid.

use bevy::prelude::Resource;

#[derive(Resource, Clone, Debug)]
pub struct Tuning {
    pub tile_size: f32,
    pub arena_columns: i32,
    pub arena_rows: i32,
    pub pixels_per_tile: i32,
    pub gravity: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            tile_size: 32.0,
            arena_columns: 16,
            arena_rows: 10,
            pixels_per_tile: 16,
            gravity: 1400.0,
        }
    }
}

impl Tuning {
    pub fn pixel_size(&self) -> f32 {
        self.tile_size / self.pixels_per_tile as f32
    }
}
