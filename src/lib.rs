//! Standalone material simulation extracted from Penguin.

pub mod noise;
pub mod simulation;
pub mod tuning;
pub mod world;

pub use simulation::{Region, Scene, Simulation, material_named};
pub use tuning::Tuning;
pub use world::{GridPosition, LevelGrid, TerrainKind};

macro_rules! timed {
    ($name:expr, $body:expr) => {{ $body }};
}
pub(crate) use timed;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod dev {
    pub mod profile {
        pub fn add(_name: &str, _elapsed: std::time::Duration) {}
    }
}
