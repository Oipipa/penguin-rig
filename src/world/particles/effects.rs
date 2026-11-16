//! Particles made by what happens in the world: reactions bubbling,
//! fizzing and glinting, puffs of smoke and steam, bubbles, sparks, and
//! emissions sent by other systems.

use super::*;

pub(in crate::world) fn receive_particles(
    mut emissions: MessageReader<ParticleEmission>,
    mut grid: ResMut<LevelGrid>,
) {
    for emission in emissions.read() {
        if let ParticleKind::Puff(kind) = emission.kind {
            grid.puff(emission.position, kind, emission.energy);
            continue;
        }
        grid.emit_mote(emission);
    }
}

impl LevelGrid {
    pub(in crate::world) fn reaction_bubble(&mut self, position: Vec2, color: [u8; 3]) {
        if self.motes.len() >= REACTION_MOTES
            || !self
                .material_at(position)
                .is_some_and(TerrainKind::is_liquid)
        {
            return;
        }
        let dice = self.roll(self.world_to_cell(position), 68);
        self.motes.push(Mote {
            position,
            previous: position,
            velocity: Vec2::new((dice % 41) as f32 - 20.0, 18.0 + ((dice >> 8) % 30) as f32),
            kind: ParticleKind::ReactionBubble(color),
            energy: 30.0 + ((dice >> 16) % 30) as f32,
            phase: if (dice >> 24).is_multiple_of(3) {
                1.0
            } else {
                0.0
            },
        });
        self.dirty = true;
    }

    /// A reaction front glowing in place for `ticks` (see
    /// [`ParticleKind::Sizzle`]). Held to half the mote budget, so a
    /// spill of acid never crowds out sparks and embers.
    pub(in crate::world) fn sizzle(&mut self, position: Vec2, color: [u8; 3], ticks: f32) {
        if self.motes.len() >= REACTION_MOTES / 2 {
            return;
        }
        self.motes.push(Mote {
            position,
            previous: position,
            velocity: Vec2::ZERO,
            kind: ParticleKind::Sizzle(color),
            energy: ticks,
            phase: 0.0,
        });
        self.dirty = true;
    }

    /// Cosmetic reaction light. It has no effect on matter or heat, and the
    /// global mote cap bounds its cost even during a large acid spill.
    pub(in crate::world) fn reaction_glint(&mut self, cell: GridPosition, color: [u8; 3]) {
        if self.motes.len() >= REACTION_MOTES {
            return;
        }
        let dice = self.roll(cell, 65);
        let position = self.cell_to_world(cell);
        self.motes.push(Mote {
            position,
            previous: position,
            velocity: Vec2::new((dice % 81) as f32 - 40.0, 30.0 + ((dice >> 8) % 65) as f32),
            kind: ParticleKind::Reaction(color),
            energy: 56.0 + ((dice >> 16) % 36) as f32,
            phase: 0.0,
        });
        self.dirty = true;
    }

    /// Release one real cell of gas at `position` if the cell there is open
    /// air: it then rises, burns out or thins like any other gas.
    pub fn puff(&mut self, position: Vec2, kind: TerrainKind, temperature: f32) {
        let cell = self.world_to_cell(position);
        let Some(i) = self.index(cell) else { return };
        if !kind.is_gas() || self.cells[i].is_some() || self.grain_at(cell) {
            return;
        }
        self.put(cell, Some(kind));
        if temperature > 0.0 {
            self.temperature[i] = temperature;
            self.track_heat(i);
        }
    }

    /// Release a cosmetic bubble (see [`ParticleKind::Bubble`]).
    pub fn bubble(&mut self, position: Vec2, velocity: Vec2, lifetime: f32) {
        if self.cells.is_empty() || self.motes.len() >= MAX_MOTES {
            return;
        }
        self.motes.push(Mote {
            position,
            previous: position,
            velocity,
            kind: ParticleKind::Bubble,
            energy: lifetime,
            phase: 0.0,
        });
        self.dirty = true;
    }

    /// Release one mote, if there is room for it. A spark remembers the
    /// life it started with (its phase), to know how far it has cooled.
    pub fn emit_mote(&mut self, emission: &ParticleEmission) {
        if self.cells.is_empty() || self.motes.len() >= MAX_MOTES {
            return;
        }
        let phase = match emission.kind {
            ParticleKind::Spark(_) | ParticleKind::Swallowed(_) => emission.energy.max(1.0),
            _ => 0.0,
        };
        self.motes.push(Mote {
            position: emission.position,
            previous: emission.position,
            velocity: emission.velocity,
            kind: emission.kind,
            energy: emission.energy,
            phase,
        });
        self.dirty = true;
    }

    /// Release a spark of `glow` living `ticks`.
    pub fn spark(&mut self, position: Vec2, velocity: Vec2, glow: Glow, ticks: f32) {
        self.emit_mote(&ParticleEmission {
            position,
            velocity,
            kind: ParticleKind::Spark(glow),
            energy: ticks,
        });
    }
}
