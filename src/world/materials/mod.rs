//! What every material is: its properties, how it responds to heat and
//! contact, how it looks (`look`) and what it reacts with (`reactions`,
//! the `REACTIONS` table).

mod look;
pub mod reactions;
mod table;

pub use crate::noise::{noise, value_noise, worley};

use reactions::REAGENT_FLAGS;
pub use reactions::{Product, Reaction, ReactionEffect};
pub use table::{Liquid, MATERIALS, MaterialDef, Phase, WATERY};

/// How a liquid moves. Water runs thin and quick; oil is viscous and lazy.
#[derive(Clone, Copy, Debug)]
pub struct LiquidStyle {
    /// Cells a resting surface cell looks along for somewhere lower to go.
    pub reach: i32,
    /// Sideways speed (cells/tick) of a loaded column spilling through an opening.
    pub spread: f32,
    /// Extra spill speed per sqrt(cell) of liquid above: deep breaches gush.
    pub head: f32,
    /// Fraction of sideways momentum kept per tick while gliding unloaded.
    pub damping: f32,
    /// Air drag applied to a falling material cell each fixed tick. The speed
    /// term gives a smooth terminal velocity instead of hitting the grid cap.
    pub air_drag: f32,
    pub air_drag_speed: f32,
    /// Relative density on Noita's scale x10 (oil 10, acid 29,
    /// water 40, blood 41). Heavier liquids sink beneath lighter ones; the
    /// closer two densities are, the slower they trade places (see
    /// [`TerrainKind::sink_chance`]).
    pub density: u8,
}

/// How a grain of debris behaves when it strikes something.
#[derive(Clone, Copy, Debug)]
pub struct Bounce {
    /// Fraction of the normal speed returned by the impact.
    pub restitution: f32,
    /// Fraction of the tangential speed kept while scraping along the surface.
    pub friction: f32,
    /// Below this speed (world px/s) the grain comes to rest.
    pub settle_speed: f32,
}

/// Relative, game-scale thermal properties. Heat transfer conserves energy
/// between cells; phase changes and combustion deliberately use simpler rules.
#[derive(Clone, Copy, Debug)]
pub struct Thermal {
    pub capacity: f32,
    pub conductivity: f32,
}

/// Materials share collision, thermal, blast, and rendering properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TerrainKind {
    Soil,
    Stone,
    Bedrock,
    Sand,
    Rubble,
    Wood,
    Coal,
    Ice,
    Water,
    Oil,
    /// Green and corrosive: converts corrodible matter, including water and
    /// ice, into flammable fumes and slowly fumes away at an exposed edge.
    Acid,
    /// Red water: a little heavier than water, mixes with it, stains solids.
    Blood,
    Fire,
    Smoke,
    Steam,
    /// Flammable gas given off where acid eats something; flashes into flame.
    Fumes,
    Ash,
    /// Dressed stone blocks: the lair's columns, floors, altars and tablets.
    /// As hard as rock, but cut clean, so it wears no crust of pebbles.
    Masonry,
    /// Raw meat: what is left of a creature that did not all turn to blood.
    /// It falls and heaps like soil (it does not slide), acid eats it
    /// readily and fire chars it to ash.
    Flesh,
    /// Molten rock: a permanent heat source that flows and burns on contact.
    Lava,
    /// Blood-cooled lava; unlike ordinary rock it resists melting back.
    VolcanicRock,
    /// Powder snow: light, loose, white. It drifts, holds a steeper bank
    /// than sand, blows off crests in a gust, takes a footprint, and melts
    /// into water at the first warmth; where it meets water the two freeze
    /// together into ice.
    Snow,
    /// Iron: plate, chains, pipes. Rusts, conducts heat, melts very hot.
    Metal,
    /// Iron eaten by water or acid: a loose orange powder.
    Rust,
    /// Brittle, clear, acid-proof: crystals and panes; sand that lava touched.
    Glass,
    /// Bones: heaps loose, chars in a hot fire.
    Bone,
    /// The caves' mushrooms: soft, burns when dried, grows in water.
    Fungus,
    /// Cobweb: tears at a touch, flashes into flame at the least heat.
    Web,
}

/// Water, blood and acid boil here.
pub const BOILING_POINT: f32 = 110.0;
/// The dried-blood tint a fully stained solid takes on.
pub const BLOOD_STAIN: [f32; 3] = [86.0, 12.0, 18.0];
/// Stain (0..=255) a solid can carry; it never quite hides the material.
pub const MAX_STAIN: u8 = 200;

impl TerrainKind {
    /// Every material, in declaration order (`ALL[kind as usize] == kind`).
    pub const ALL: [TerrainKind; 28] = [
        Self::Soil,
        Self::Stone,
        Self::Bedrock,
        Self::Sand,
        Self::Rubble,
        Self::Wood,
        Self::Coal,
        Self::Ice,
        Self::Water,
        Self::Oil,
        Self::Acid,
        Self::Blood,
        Self::Fire,
        Self::Smoke,
        Self::Steam,
        Self::Fumes,
        Self::Ash,
        Self::Masonry,
        Self::Flesh,
        Self::Lava,
        Self::VolcanicRock,
        Self::Snow,
        Self::Metal,
        Self::Rust,
        Self::Glass,
        Self::Bone,
        Self::Fungus,
        Self::Web,
    ];

    /// Its row of properties (see `table.rs`).
    #[inline]
    pub fn def(self) -> &'static MaterialDef {
        &MATERIALS[self as usize]
    }

    pub fn thermal(self) -> Thermal {
        self.def().thermal
    }

    /// Casing contact: restitution and tangential drag per second.
    pub fn casing_contact(self) -> (f32, f32) {
        self.def().casing_contact
    }

    pub fn is_liquid(self) -> bool {
        matches!(self.def().phase, Phase::Liquid(_))
    }
    pub fn is_gas(self) -> bool {
        matches!(self.def().phase, Phase::Gas)
    }
    /// Water-based liquids: they run like water, mix with each other and
    /// share one pool's pressure. Oil does neither.
    pub fn is_aqueous(self) -> bool {
        matches!(
            self.def().phase,
            Phase::Liquid(Liquid { aqueous: true, .. })
        )
    }
    /// Liquids that belong to one body of liquid when they touch: they share
    /// its pressure. Water-based liquids pool together; oil only with oil.
    pub fn pools_with(self, other: Self) -> bool {
        self == other || (self.is_aqueous() && other.is_aqueous())
    }
    /// Materials with chemistry of their own every tick, hot or not: every
    /// reagent in [`REACTIONS`], plus blood, which stains what it touches
    /// (see `world/reactions/`).
    pub fn reacts(self) -> bool {
        self == Self::Blood || REAGENT_FLAGS[self as usize]
    }
    /// Liquids that smother flame and keep fuel (or a body) from catching.
    /// Acid does not: fire boils it into flammable fumes instead.
    pub fn quenches(self) -> bool {
        matches!(
            self.def().phase,
            Phase::Liquid(Liquid { quenches: true, .. })
        )
    }
    /// What a liquid boils into at `BOILING_POINT`, if it boils.
    pub fn vapor(self) -> Option<Self> {
        match self.def().phase {
            Phase::Liquid(liquid) => liquid.vapor,
            _ => None,
        }
    }
    /// Chance per tick (0..=1) that this liquid, resting on `other`, trades
    /// places with it. Clearly heavier liquids always sink; near-equal ones
    /// (blood in water, water under acid) sink slowly and mix on the way,
    /// the way Noita swaps liquids by density difference.
    pub fn sink_chance(self, other: Self) -> f32 {
        if !self.is_liquid() || !other.is_liquid() || self == other {
            return 0.0;
        }
        let difference = self.liquid_style().density as f32 - other.liquid_style().density as f32;
        (difference / 8.0).clamp(0.0, 1.0)
    }
    pub fn is_solid(self) -> bool {
        !self.is_liquid() && !self.is_gas()
    }
    pub fn is_powder(self) -> bool {
        matches!(self.def().phase, Phase::Powder { .. })
    }
    /// Powders that hold together: they fall straight down but do not
    /// slide off a heap (soil, and a carcass of flesh).
    pub fn is_cohesive(self) -> bool {
        matches!(self.def().phase, Phase::Powder { cohesive: true })
    }
    /// How it runs as a liquid (as water does, if it is not one).
    pub fn liquid_style(self) -> LiquidStyle {
        match self.def().phase {
            Phase::Liquid(liquid) => liquid.style,
            _ => WATERY,
        }
    }
    pub fn bounce(self) -> Bounce {
        self.def().bounce
    }
    pub fn loose(self) -> Self {
        self.def().loose
    }
    pub fn toughness(self) -> f32 {
        self.def().toughness
    }
    pub fn blast_resistance(self) -> u32 {
        self.def().blast_resistance
    }
    pub fn ignition(self) -> f32 {
        self.def().ignition
    }
    pub fn lifetime(self) -> u16 {
        self.def().lifetime
    }
    /// Loose debris a body wading through it kicks along (see
    /// `LevelGrid::stir_loose`): shards, scrap, rust, bone, mushroom, silk
    /// and flesh. Ground (sand, soil, snow, ash) is walked on, not kicked.
    pub fn stirs(self) -> bool {
        matches!(
            self,
            Self::Glass
                | Self::Metal
                | Self::Rust
                | Self::Bone
                | Self::Fungus
                | Self::Web
                | Self::Flesh
        )
    }
    /// The temperature it melts at and what into, if it melts.
    pub fn melts(self) -> Option<(f32, Self)> {
        self.def().melts
    }
}
