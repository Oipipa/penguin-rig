//! A gas cell's step: rising, drifting and thinning out.

use super::*;

impl LevelGrid {
    pub(super) fn step_gas(&mut self, mut cell: GridPosition, kind: TerrainKind) {
        let mut i = self.index(cell).unwrap();
        self.life[i] = self.life[i].saturating_sub(1);
        if (self.life[i] == 0 && kind != TerrainKind::Steam) || cell.y == self.height - 1 {
            let next = if cell.y == self.height - 1 {
                None
            } else {
                match kind {
                    // Most tongues of flame just burn out; some leave a wisp of smoke.
                    TerrainKind::Fire => self
                        .roll(cell, 8)
                        .is_multiple_of(3)
                        .then_some(TerrainKind::Smoke),
                    _ => None,
                }
            };
            self.put(cell, next);
            return;
        }
        let t = self.tick as f32;
        let curl = (cell.x as f32 * 0.061 + cell.y as f32 * 0.029 + t * 0.033).sin() * 0.48
            + (cell.y as f32 * 0.087 - t * 0.019).sin() * 0.22;
        let jitter = (self.roll(cell, 9) % 1024) as f32 / 512.0 - 1.0;
        let (target, response) = match kind {
            TerrainKind::Fire => {
                // Nearby fire shares a curl; independent white noise made a
                // plume look like sparks instead of connected tongues.
                let heat = ((self.temperature[i] - 150.0) / 570.0).clamp(0.0, 1.0);
                (
                    Vec2::new(curl * 0.75 + jitter * 0.12, 0.18 + heat * 0.62),
                    0.2,
                )
            }
            TerrainKind::Steam => (Vec2::new(curl * 0.8 + jitter * 0.2, 0.6), 0.12),
            // Fumes are heavy for a gas: they drift and pool under ceilings.
            TerrainKind::Fumes => (Vec2::new(curl * 0.9 + jitter * 0.3, 0.2), 0.08),
            _ => (Vec2::new(curl + jitter * 0.15, 0.36), 0.1),
        };
        self.flow[i] = self.flow[i].lerp(target, response);
        self.phase[i] += self.flow[i];
        let dx = self.phase[i].x.trunc().clamp(-1.0, 1.0) as i32;
        let dy = self.phase[i].y.floor().clamp(0.0, 2.0) as i32;
        self.phase[i] -= Vec2::new(dx as f32, dy as f32);
        let side = if self.flow[i].x >= 0.0 { 1 } else { -1 };
        for step in 0..dy.max(i32::from(dx != 0)) {
            let rise = i32::from(step < dy);
            let drift = if step == 0 { dx } else { 0 };
            let candidates = [(drift, rise), (side, rise), (-side, rise), (side, 0)];
            let mut moved = false;
            for (x, y) in candidates {
                if x == 0 && y == 0 {
                    continue;
                }
                let next = GridPosition {
                    x: cell.x + x,
                    y: cell.y + y,
                };
                let Some(j) = self.index(next) else { continue };
                if self.grain_at(next)
                    || (x != 0
                        && self.solid(GridPosition {
                            x: next.x,
                            y: cell.y,
                        }))
                {
                    continue;
                }
                let displace = y > 0
                    && ((self.cells[j].is_some_and(TerrainKind::is_gas)
                        && self.temperature[i] > self.temperature[j] + 80.0)
                        // Vapor bubbles lift through liquid without deleting it.
                        || (matches!(kind, TerrainKind::Steam | TerrainKind::Fumes)
                            && self.cells[j].is_some_and(TerrainKind::is_liquid)));
                if self.cells[j].is_none() || displace {
                    self.swap_cells(cell, next);
                    cell = next;
                    i = j;
                    moved = true;
                    break;
                }
            }
            if !moved {
                self.flow[i].x *= -0.4;
                break;
            }
        }
        if kind == TerrainKind::Fire {
            // Tongues of flame shed heat quickly as they rise away from the fuel.
            self.temperature[i] += (20.0 - self.temperature[i]) * 0.025;
            self.track_heat(i);
        }
        self.dirty = true;
    }
}
