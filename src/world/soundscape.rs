//! What the material world sounds like: the simulation notes each thing
//! that makes a noise (a flame, a bite of acid, a drop landing, stone
//! settling) where the camera can see it, and the audio drains the tally
//! every frame to set the fire's crackle, the acid's fizz, the water's rush,
//! and to drop the odd plip, splash or clatter where they happened.

use super::*;

/// A kind of noise the material makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Noise {
    /// A flame.
    Burning,
    /// Acid eating something solid.
    Corrode,
    /// Acid meeting water or blood.
    Mix,
    /// Liquid falling and landing: a pour, a stream, a spill.
    Flow,
    /// Liquid boiling off.
    Boil,
    /// A drop landing in a pool.
    Plip,
    /// A drop landing on rock.
    Pat,
    /// A landing pour splashing back up.
    Splash,
    /// A loose solid striking the ground.
    Debris,
    /// Fumes flashing into flame.
    Ignite,
    /// Flame put out by water, or water flashed to steam by it.
    Quench,
}

const NOISES: usize = 11;

/// How much of a noise there was, and where.
#[derive(Clone, Copy, Default, Debug)]
pub struct Tally {
    pub count: u32,
    /// Sum of the positions (world units), for the middle of it all.
    pub sum: Vec2,
    /// The most recent one.
    pub last: Vec2,
    /// Sum of how hard each one was.
    pub strength: f32,
}

impl Tally {
    pub fn middle(&self) -> Vec2 {
        if self.count == 0 {
            self.last
        } else {
            self.sum / self.count as f32
        }
    }
}

/// The noises of the ticks since the audio last listened.
#[derive(Clone, Default, Debug)]
pub struct Soundscape {
    tallies: [Tally; NOISES],
}

impl Soundscape {
    pub fn get(&self, noise: Noise) -> Tally {
        self.tallies[noise as usize]
    }
}

impl LevelGrid {
    /// Note a noise at `cell`, if it is where the camera looks.
    #[inline]
    pub(super) fn hear(&mut self, noise: Noise, cell: GridPosition, strength: f32) {
        if self.view.is_some_and(|view| !view.contains(cell)) {
            return;
        }
        let at = self.cell_to_world(cell);
        let tally = &mut self.soundscape.tallies[noise as usize];
        tally.count += 1;
        tally.sum += at;
        tally.last = at;
        tally.strength += strength;
    }

    /// Everything heard since the last call, clearing the tally.
    pub fn take_soundscape(&mut self) -> Soundscape {
        std::mem::take(&mut self.soundscape)
    }
}
