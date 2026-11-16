//! The ambient life of the air: fireflies over grass, dust drifting in
//! caves, drips from damp roofs, bubbles rising through water.

use super::*;

/// How many ambient motes of each kind live near the player at once.
pub(super) const FIREFLIES: usize = 18;

pub(super) const DRIFT: usize = 40;

pub(super) const DRIPS: usize = 10;

/// Lifetime an ambient mote starts with (in 1/60 s), set by its phase
/// (0..10), so its age is known for fading in.
pub(in crate::world) fn ambient_lifetime(kind: ParticleKind, phase: f32) -> f32 {
    match kind {
        ParticleKind::Firefly => 480.0 + phase * 24.0,
        _ => 240.0 + phase * 24.0,
    }
}

/// Fade (0..=1) of an ambient mote: in over its first and out over its last
/// second.
pub(in crate::world) fn ambient_fade(mote: &Mote) -> f32 {
    let age = ambient_lifetime(mote.kind, mote.phase) - mote.energy;
    (age / 60.0).clamp(0.0, 1.0) * (mote.energy / 60.0).clamp(0.0, 1.0)
}

impl LevelGrid {
    /// Fireflies wander on a slow noise field and keep a few cells above the
    /// ground; dust drifts. Both die in liquid and never touch the world.
    pub(super) fn step_ambient_mote(&mut self, mote: &mut Mote, dt: f32) -> bool {
        mote.energy -= dt * 60.0;
        if mote.energy <= 0.0
            || self
                .material_at(mote.position)
                .is_some_and(|kind| !kind.is_gas())
        {
            return false;
        }
        let t = self.tick as f32 / 60.0;
        if matches!(mote.kind, ParticleKind::Firefly) {
            let angle = crate::world::materials::value_noise(
                mote.position.x / 60.0 + mote.phase,
                mote.position.y / 60.0 + t * 0.15,
                7,
            ) * std::f32::consts::TAU
                * 2.0;
            let target = Vec2::new(angle.cos(), angle.sin() * 0.6) * 16.0;
            mote.velocity = mote.velocity.lerp(target, 0.04);
            let cell = self.world_to_cell(mote.position);
            let ground = (1..=16).find(|dy| {
                self.get(GridPosition {
                    x: cell.x,
                    y: cell.y - dy,
                })
                .is_some_and(|kind| !kind.is_gas())
            });
            match ground {
                Some(dy) if dy <= 2 => mote.velocity.y += 2.0,
                None => mote.velocity.y -= 1.5,
                _ => {}
            }
        } else {
            let dice = self.roll(self.world_to_cell(mote.position), 71);
            let kick = Vec2::new(
                (dice % 100) as f32 / 100.0 - 0.5,
                ((dice >> 8) % 100) as f32 / 100.0 - 0.55,
            );
            mote.velocity = (mote.velocity + kick * 3.0) * 0.96;
        }
        let next = mote.position + mote.velocity * dt;
        if self.material_at(next).is_some_and(|kind| !kind.is_gas())
            || self.index(self.world_to_cell(next)).is_none()
        {
            mote.velocity = -mote.velocity * 0.5;
        } else {
            mote.position = next;
        }
        true
    }

    /// Keep a little ambient life near the player: fireflies over grass under
    /// the open sky, dust hanging in caves (none of it in the Void).
    pub(in crate::world) fn step_ambient(&mut self, player: Option<Vec2>) {
        let Some(player) = player else { return };
        // The Void has no air to speak of: nothing drifts, drips or glows
        // in it but what the story puts there.
        if false {
            return;
        }
        if self.cells.is_empty() || self.motes.len() >= MAX_MOTES - 64 {
            return;
        }
        let (fireflies, drift, drips) =
            self.motes
                .iter()
                .fold((0, 0, 0), |(f, d, w), mote| match mote.kind {
                    ParticleKind::Firefly => (f + 1, d, w),
                    ParticleKind::Drift => (f, d + 1, w),
                    ParticleKind::Drip => (f, d, w + 1),
                    _ => (f, d, w),
                });
        let center = self.world_to_cell(player);
        let dice = self.roll(center, 73);
        if fireflies < FIREFLIES && self.tick.is_multiple_of(9) {
            let x = center.x + (dice % 360) as i32 - 180;
            let top = (0..self.height).rev().find(|&y| {
                self.get(GridPosition { x, y })
                    .is_some_and(|kind| !kind.is_gas())
            });
            if let Some(top) = top
                && self.get(GridPosition { x, y: top }) == Some(TerrainKind::Soil)
                && top + 12 < self.height
            {
                let cell = GridPosition {
                    x,
                    y: top + 3 + ((dice >> 9) % 9) as i32,
                };
                let phase = ((dice >> 3) % 1000) as f32 / 100.0;
                self.motes.push(Mote {
                    position: self.cell_to_world(cell),
                    previous: self.cell_to_world(cell),
                    velocity: Vec2::ZERO,
                    kind: ParticleKind::Firefly,
                    energy: ambient_lifetime(ParticleKind::Firefly, phase),
                    phase,
                });
            }
        }
        // Damp spots in cave roofs drip, always the same spots.
        if drips < DRIPS && self.tick.is_multiple_of(5) {
            let x = center.x + ((dice >> 3) % 400) as i32 - 200;
            let from = center.y + ((dice >> 12) % 60) as i32;
            let roof = (from..from + 120).find(|&y| {
                self.get(GridPosition { x, y })
                    .is_some_and(|kind| kind.is_solid())
            });
            if let Some(roof) = roof {
                let under = GridPosition { x, y: roof - 1 };
                let damp =
                    crate::world::materials::value_noise(x as f32 / 9.0, roof as f32 / 7.0, 419);
                if damp > 0.68
                    && self.get(under).is_none()
                    && self.render_cache.sky_share(&self.chunks, under) < 0.3
                {
                    let at = self.cell_to_world(under);
                    self.motes.push(Mote {
                        position: at,
                        previous: at,
                        velocity: Vec2::ZERO,
                        kind: ParticleKind::Drip,
                        energy: 1.0,
                        phase: 0.0,
                    });
                }
            }
        }
        if drift < DRIFT && self.tick.is_multiple_of(4) {
            let cell = GridPosition {
                x: center.x + (dice % 240) as i32 - 120,
                y: center.y + ((dice >> 8) % 140) as i32 - 70,
            };
            let open = self.index(cell).is_some() && self.get(cell).is_none();
            if open && self.render_cache.sky_share(&self.chunks, cell) < 0.3 {
                let phase = ((dice >> 16) % 1000) as f32 / 100.0;
                self.motes.push(Mote {
                    position: self.cell_to_world(cell),
                    previous: self.cell_to_world(cell),
                    velocity: Vec2::ZERO,
                    kind: ParticleKind::Drift,
                    energy: ambient_lifetime(ParticleKind::Drift, phase),
                    phase,
                });
            }
        }
    }

    /// Bubbles rise, drag, and wobble; they pop on reaching air and slide
    /// along anything solid above them.
    pub(super) fn step_bubble(&mut self, mote: &mut Mote, dt: f32) -> bool {
        if !self
            .material_at(mote.position)
            .is_some_and(TerrainKind::is_liquid)
        {
            return false;
        }
        mote.energy -= dt * 30.0;
        if mote.energy <= 0.0 {
            return false;
        }
        mote.velocity.y += 460.0 * dt;
        mote.velocity *= 1.0 / (1.0 + dt * 5.0);
        let wobble = (mote.energy * 0.4 + mote.position.y * 0.25).sin() * 22.0;
        let next = mote.position + (mote.velocity + Vec2::X * wobble) * dt;
        if self.material_at(next).is_some_and(TerrainKind::is_solid) {
            mote.velocity.y = 0.0;
            mote.velocity.x += wobble.signum() * 30.0;
        } else {
            mote.position = next;
        }
        true
    }
}
