//! Public, fixed-tick API for the standalone material simulator.

use crate::{GridPosition, LevelGrid, TerrainKind, Tuning};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scene {
    pub width: i32,
    pub height: i32,
    pub gravity: f32,
    pub tick_hz: f32,
    pub speed: f32,
    pub zoom: f32,
    pub weather: bool,
    pub regions: Vec<Region>,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            width: 256,
            height: 160,
            gravity: 1400.0,
            tick_hz: 60.0,
            speed: 1.0,
            zoom: 3.0,
            weather: false,
            regions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub material: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

pub fn material_named(name: &str) -> Option<TerrainKind> {
    let key = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    TerrainKind::ALL
        .into_iter()
        .find(|kind| format!("{kind:?}").to_lowercase() == key)
}

impl Scene {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let scene: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        scene.validate()?;
        Ok(scene)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_toml(&text)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        self.validate()?;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, text).map_err(|e| e.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(32..=512).contains(&self.width) || !(32..=512).contains(&self.height) {
            return Err("width and height must each be between 32 and 512 cells".into());
        }
        if !self.gravity.is_finite() || !(0.0..=5000.0).contains(&self.gravity) {
            return Err("gravity must be finite and between 0 and 5000".into());
        }
        if !self.tick_hz.is_finite() || !(1.0..=240.0).contains(&self.tick_hz) {
            return Err("tick_hz must be finite and between 1 and 240".into());
        }
        if !self.speed.is_finite() || !(0.05..=8.0).contains(&self.speed) {
            return Err("speed must be finite and between 0.05 and 8".into());
        }
        if !self.zoom.is_finite() || !(1.0..=8.0).contains(&self.zoom) {
            return Err("zoom must be finite and between 1 and 8".into());
        }
        for (n, region) in self.regions.iter().enumerate() {
            if material_named(&region.material).is_none() {
                return Err(format!(
                    "region {} has unknown material {:?}",
                    n + 1,
                    region.material
                ));
            }
            if region.width <= 0
                || region.height <= 0
                || region.x < 0
                || region.y < 0
                || region
                    .x
                    .checked_add(region.width)
                    .is_none_or(|end| end > self.width)
                || region
                    .y
                    .checked_add(region.height)
                    .is_none_or(|end| end > self.height)
            {
                return Err(format!(
                    "region {} is outside the grid or has an invalid size",
                    n + 1
                ));
            }
            if region
                .temperature
                .is_some_and(|t| !t.is_finite() || !(-200.0..=1400.0).contains(&t))
            {
                return Err(format!("region {} has an invalid temperature", n + 1));
            }
        }
        Ok(())
    }
}

pub struct Simulation {
    grid: LevelGrid,
    initial: Scene,
    pub gravity: f32,
    pub tick_hz: f32,
    pub speed: f32,
    pub weather: bool,
    ticks: u64,
}

impl Simulation {
    pub fn new(scene: Scene) -> Result<Self, String> {
        scene.validate()?;
        let tuning = Tuning {
            tile_size: 2.0,
            arena_columns: scene.width,
            arena_rows: scene.height,
            pixels_per_tile: 1,
            gravity: scene.gravity,
        };
        let mut grid = LevelGrid::new(&tuning);
        grid.weather = scene.weather;
        if scene.weather {
            grid.view = Some(crate::world::render::CellRect {
                x0: 0,
                y0: 0,
                x1: scene.width - 1,
                y1: scene.height - 1,
            });
        }
        for region in &scene.regions {
            let kind = material_named(&region.material).unwrap();
            for y in region.y..region.y + region.height {
                for x in region.x..region.x + region.width {
                    let cell = GridPosition { x, y };
                    grid.put(cell, Some(kind));
                    if let Some(target) = region.temperature {
                        let point = grid.cell_to_world(cell);
                        grid.add_heat(point, target - grid.temperature_at(point));
                    }
                }
            }
        }
        Ok(Self {
            grid,
            gravity: scene.gravity,
            tick_hz: scene.tick_hz,
            speed: scene.speed,
            weather: scene.weather,
            initial: scene,
            ticks: 0,
        })
    }

    pub fn grid(&self) -> &LevelGrid {
        &self.grid
    }
    pub fn ticks(&self) -> u64 {
        self.ticks
    }
    pub fn get(&self, cell: GridPosition) -> Option<TerrainKind> {
        self.grid.get(cell)
    }

    pub fn paint(&mut self, cell: GridPosition, kind: Option<TerrainKind>) {
        self.grid.put(cell, kind);
    }

    pub fn heat(&mut self, cell: GridPosition, degrees: f32) {
        let point = self.grid.cell_to_world(cell);
        self.grid.add_heat(point, degrees);
    }

    pub fn set_temperature(&mut self, cell: GridPosition, temperature: f32) {
        if temperature.is_finite() && (-200.0..=1400.0).contains(&temperature) {
            let point = self.grid.cell_to_world(cell);
            self.grid
                .add_heat(point, temperature - self.grid.temperature_at(point));
        }
    }

    pub fn step(&mut self) {
        self.grid.weather = self.weather;
        self.grid.view = self.weather.then_some(crate::world::render::CellRect {
            x0: 0,
            y0: 0,
            x1: self.grid.width - 1,
            y1: self.grid.height - 1,
        });
        self.grid
            .step_materials(1.0 / self.tick_hz, self.gravity, None);
        self.ticks += 1;
    }

    pub fn step_n(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step();
        }
    }

    pub fn reset(&mut self) {
        let scene = self.initial.clone();
        *self = Self::new(scene).expect("validated initial scene");
    }

    /// Export settled cells as horizontal runs. Air and in-flight grains are omitted.
    pub fn snapshot_scene(&self) -> Scene {
        let mut scene = Scene {
            width: self.grid.width,
            height: self.grid.height,
            gravity: self.gravity,
            tick_hz: self.tick_hz,
            speed: self.speed,
            zoom: self.initial.zoom,
            weather: self.weather,
            regions: Vec::new(),
        };
        for y in 0..self.grid.height {
            let mut x = 0;
            while x < self.grid.width {
                let cell = GridPosition { x, y };
                let Some(kind) = self.grid.get(cell) else {
                    x += 1;
                    continue;
                };
                let temp = self
                    .grid
                    .index(cell)
                    .map(|i| self.grid.temperature[i])
                    .unwrap_or(20.0);
                let start = x;
                x += 1;
                while x < self.grid.width {
                    let p = GridPosition { x, y };
                    if self.grid.get(p) != Some(kind)
                        || self
                            .grid
                            .index(p)
                            .is_none_or(|i| self.grid.temperature[i].to_bits() != temp.to_bits())
                    {
                        break;
                    }
                    x += 1;
                }
                scene.regions.push(Region {
                    x: start,
                    y,
                    width: x - start,
                    height: 1,
                    material: format!("{kind:?}"),
                    temperature: Some(temp),
                });
            }
        }
        scene
    }
}
