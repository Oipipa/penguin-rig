//! Every material's properties, one row each, in `TerrainKind` order: how
//! it moves (its [`Phase`]), takes and passes heat, meets a bouncing casing
//! and a falling grain, breaks, burns and corrodes. `TerrainKind`'s
//! property methods read these rows; how a material looks is `look.rs`, and
//! what it reacts with is `REACTIONS` in `reactions.rs`.
//!
//! A new material is a `TerrainKind` variant and a row here (the compiler
//! counts the rows; `every_row_is_in_its_place` checks their order), its
//! look, and any reactions.

use super::*;
use TerrainKind::*;

/// Never ignites.
const NEVER: f32 = f32::INFINITY;

/// How a material moves.
#[derive(Clone, Copy, Debug)]
pub enum Phase {
    /// Stays where it is unless broken loose.
    Solid,
    /// Falls and heaps; a cohesive powder falls straight down but does not
    /// slide off a heap (soil, a carcass of flesh).
    Powder {
        cohesive: bool,
    },
    Liquid(Liquid),
    Gas,
}

/// What kind of liquid a material is.
#[derive(Clone, Copy, Debug)]
pub struct Liquid {
    /// Water-based: runs like water, mixes with the others and shares one
    /// pool's pressure.
    pub aqueous: bool,
    /// Smothers flame and keeps fuel (or a body) from catching.
    pub quenches: bool,
    /// What it boils into at `BOILING_POINT`, if it boils.
    pub vapor: Option<TerrainKind>,
    pub style: LiquidStyle,
}

/// One material's properties (see [`MATERIALS`]).
#[derive(Clone, Copy, Debug)]
pub struct MaterialDef {
    /// Which material the row is for: rows are read by position, and a test
    /// checks each sits at its kind's.
    pub kind: TerrainKind,
    pub phase: Phase,
    pub thermal: Thermal,
    /// A casing striking it: restitution, and tangential drag per second
    /// (time-based, so fast projectiles taking more substeps are not braked
    /// harder).
    pub casing_contact: (f32, f32),
    /// A falling grain of it striking something.
    pub bounce: Bounce,
    /// What it breaks into.
    pub loose: TerrainKind,
    /// Damage it takes to break.
    pub toughness: f32,
    /// How much blast pressure it stops.
    pub blast_resistance: u32,
    /// Temperature it catches fire at.
    pub ignition: f32,
    /// Ticks it burns (fuel) or lasts (a gas).
    pub lifetime: u16,
    /// How fast acid eats it (`MaterialTag::Corrodible`); 0 is acid-proof.
    pub corrodes: f32,
    /// The temperature it melts at, and what it melts into.
    pub melts: Option<(f32, TerrainKind)>,
}

/// How a liquid that is not one moves, if asked: as water does.
pub const WATERY: LiquidStyle = LiquidStyle {
    reach: 16,
    spread: 2.0,
    head: 0.5,
    damping: 0.82,
    air_drag: 0.055,
    air_drag_speed: 0.025,
    density: 40,
};

/// Every material's row, by `TerrainKind as usize`.
pub const MATERIALS: [MaterialDef; TerrainKind::ALL.len()] = [
    // Soil
    MaterialDef {
        kind: Soil,
        phase: Phase::Powder { cohesive: true },
        thermal: Thermal {
            capacity: 1.5,
            conductivity: 0.16,
        },
        casing_contact: (0.1, 18.0),
        bounce: Bounce {
            restitution: 0.08,
            friction: 0.45,
            settle_speed: 120.0,
        },
        loose: Sand,
        toughness: 180.0,
        blast_resistance: 60,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 1.2,
        melts: None,
    },
    // Stone
    MaterialDef {
        kind: Stone,
        phase: Phase::Solid,
        thermal: Thermal {
            capacity: 1.5,
            conductivity: 0.16,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Rubble,
        toughness: 1050.0,
        blast_resistance: 340,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 0.22,
        // Ordinary cave rock eventually joins the flow.
        melts: Some((900.0, Lava)),
    },
    // Nothing breaks it.
    // Bedrock
    MaterialDef {
        kind: Bedrock,
        phase: Phase::Solid,
        thermal: Thermal {
            capacity: 1.5,
            conductivity: 0.16,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Bedrock,
        toughness: NEVER,
        blast_resistance: u32::MAX,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 0.0,
        melts: None,
    },
    // Sand
    MaterialDef {
        kind: Sand,
        phase: Phase::Powder { cohesive: false },
        thermal: Thermal {
            capacity: 1.5,
            conductivity: 0.16,
        },
        casing_contact: (0.1, 18.0),
        bounce: Bounce {
            restitution: 0.08,
            friction: 0.45,
            settle_speed: 120.0,
        },
        loose: Sand,
        toughness: 85.0,
        blast_resistance: 35,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 1.2,
        melts: None,
    },
    // Rubble
    MaterialDef {
        kind: Rubble,
        phase: Phase::Powder { cohesive: false },
        thermal: Thermal {
            capacity: 1.5,
            conductivity: 0.16,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.32,
            friction: 0.8,
            settle_speed: 60.0,
        },
        loose: Rubble,
        toughness: 130.0,
        blast_resistance: 60,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 0.75,
        melts: None,
    },
    // Wood
    MaterialDef {
        kind: Wood,
        phase: Phase::Solid,
        thermal: Thermal {
            capacity: 1.0,
            conductivity: 0.08,
        },
        casing_contact: (0.24, 9.0),
        bounce: Bounce {
            restitution: 0.24,
            friction: 0.7,
            settle_speed: 70.0,
        },
        loose: Wood,
        toughness: 230.0,
        blast_resistance: 80,
        ignition: 210.0,
        lifetime: 420,
        corrodes: 0.75,
        melts: None,
    },
    // Coal
    MaterialDef {
        kind: Coal,
        phase: Phase::Solid,
        thermal: Thermal {
            capacity: 1.0,
            conductivity: 0.08,
        },
        casing_contact: (0.24, 9.0),
        bounce: Bounce {
            restitution: 0.24,
            friction: 0.7,
            settle_speed: 70.0,
        },
        loose: Coal,
        toughness: 380.0,
        blast_resistance: 130,
        ignition: 310.0,
        lifetime: 650,
        corrodes: 0.35,
        melts: None,
    },
    // Ice
    MaterialDef {
        kind: Ice,
        phase: Phase::Solid,
        thermal: Thermal {
            capacity: 2.0,
            conductivity: 0.22,
        },
        casing_contact: (0.48, 0.7),
        bounce: Bounce {
            restitution: 0.32,
            friction: 0.8,
            settle_speed: 60.0,
        },
        loose: Ice,
        toughness: 200.0,
        blast_resistance: 80,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 1.0,
        melts: None,
    },
    // Runs fast and thin, Noita style: a breach empties quickly and a
    // spill races across the floor.
    // Water
    MaterialDef {
        kind: Water,
        phase: Phase::Liquid(Liquid {
            aqueous: true,
            quenches: true,
            vapor: Some(Steam),
            style: LiquidStyle {
                reach: 16,
                spread: 2.0,
                head: 0.5,
                damping: 0.82,
                air_drag: 0.055,
                air_drag_speed: 0.025,
                density: 40,
            },
        }),
        thermal: Thermal {
            capacity: 4.0,
            conductivity: 0.32,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Water,
        toughness: 60.0,
        blast_resistance: 155,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 1.0,
        melts: None,
    },
    // Oil
    MaterialDef {
        kind: Oil,
        phase: Phase::Liquid(Liquid {
            aqueous: false,
            quenches: false,
            vapor: None,
            style: LiquidStyle {
                reach: 8,
                spread: 0.65,
                head: 0.16,
                damping: 0.72,
                air_drag: 0.085,
                air_drag_speed: 0.035,
                density: 10,
            },
        }),
        thermal: Thermal {
            capacity: 1.8,
            conductivity: 0.09,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Oil,
        toughness: 60.0,
        blast_resistance: 70,
        ignition: 150.0,
        lifetime: 180,
        corrodes: 0.0,
        melts: None,
    },
    // Acid
    MaterialDef {
        kind: Acid,
        phase: Phase::Liquid(Liquid {
            aqueous: true,
            quenches: false,
            vapor: Some(Fumes),
            style: LiquidStyle {
                reach: 16,
                spread: 2.0,
                head: 0.5,
                damping: 0.82,
                air_drag: 0.055,
                air_drag_speed: 0.025,
                density: 29,
            },
        }),
        thermal: Thermal {
            capacity: 3.6,
            conductivity: 0.3,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Acid,
        toughness: 60.0,
        blast_resistance: 155,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 0.0,
        melts: None,
    },
    // Runs like water, a little thicker: it pools in a puddle rather
    // than racing off as a film.
    // Blood
    MaterialDef {
        kind: Blood,
        phase: Phase::Liquid(Liquid {
            aqueous: true,
            quenches: true,
            vapor: Some(Steam),
            style: LiquidStyle {
                reach: 10,
                spread: 1.4,
                head: 0.4,
                damping: 0.76,
                air_drag: 0.06,
                air_drag_speed: 0.027,
                density: 41,
            },
        }),
        thermal: Thermal {
            capacity: 3.8,
            conductivity: 0.3,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Blood,
        toughness: 60.0,
        blast_resistance: 155,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 1.0,
        melts: None,
    },
    // Fire
    MaterialDef {
        kind: Fire,
        phase: Phase::Gas,
        thermal: Thermal {
            capacity: 0.3,
            conductivity: 0.12,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Fire,
        toughness: 60.0,
        blast_resistance: 5,
        ignition: NEVER,
        lifetime: 22,
        corrodes: 0.0,
        melts: None,
    },
    // Smoke
    MaterialDef {
        kind: Smoke,
        phase: Phase::Gas,
        thermal: Thermal {
            capacity: 0.25,
            conductivity: 0.025,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Smoke,
        toughness: 60.0,
        blast_resistance: 5,
        ignition: NEVER,
        lifetime: 150,
        corrodes: 0.0,
        melts: None,
    },
    // Steam
    MaterialDef {
        kind: Steam,
        phase: Phase::Gas,
        thermal: Thermal {
            capacity: 0.8,
            conductivity: 0.06,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Steam,
        toughness: 60.0,
        blast_resistance: 5,
        ignition: NEVER,
        lifetime: 320,
        corrodes: 0.0,
        melts: None,
    },
    // Fumes
    MaterialDef {
        kind: Fumes,
        phase: Phase::Gas,
        thermal: Thermal {
            capacity: 0.3,
            conductivity: 0.04,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Fumes,
        toughness: 60.0,
        blast_resistance: 5,
        ignition: 170.0,
        lifetime: 300,
        corrodes: 0.0,
        melts: None,
    },
    // Ash
    MaterialDef {
        kind: Ash,
        phase: Phase::Powder { cohesive: false },
        thermal: Thermal {
            capacity: 1.5,
            conductivity: 0.16,
        },
        casing_contact: (0.1, 18.0),
        bounce: Bounce {
            restitution: 0.08,
            friction: 0.45,
            settle_speed: 120.0,
        },
        loose: Ash,
        toughness: 35.0,
        blast_resistance: 5,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 1.8,
        melts: None,
    },
    // Masonry
    MaterialDef {
        kind: Masonry,
        phase: Phase::Solid,
        thermal: Thermal {
            capacity: 1.5,
            conductivity: 0.16,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Rubble,
        toughness: 900.0,
        blast_resistance: 300,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 0.18,
        melts: None,
    },
    // Meat lands with a slap and stays put; it chars to ash.
    // Flesh
    MaterialDef {
        kind: Flesh,
        phase: Phase::Powder { cohesive: true },
        thermal: Thermal {
            capacity: 2.6,
            conductivity: 0.1,
        },
        casing_contact: (0.05, 14.0),
        bounce: Bounce {
            restitution: 0.04,
            friction: 0.3,
            settle_speed: 150.0,
        },
        loose: Flesh,
        toughness: 70.0,
        blast_resistance: 25,
        ignition: 240.0,
        lifetime: 260,
        corrodes: 1.5,
        melts: None,
    },
    // Noita's density is 6.0, above water's 4.0: it runs nearly as fast
    // as water, but its surface holds together a little more.
    // Lava
    MaterialDef {
        kind: Lava,
        phase: Phase::Liquid(Liquid {
            aqueous: false,
            quenches: false,
            vapor: None,
            style: LiquidStyle {
                reach: 14,
                spread: 1.8,
                head: 0.46,
                damping: 0.8,
                air_drag: 0.06,
                air_drag_speed: 0.027,
                density: 60,
            },
        }),
        thermal: Thermal {
            capacity: 3.0,
            conductivity: 0.38,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: Lava,
        toughness: 60.0,
        blast_resistance: 155,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 0.0,
        melts: None,
    },
    // VolcanicRock
    MaterialDef {
        kind: VolcanicRock,
        phase: Phase::Solid,
        thermal: Thermal {
            capacity: 1.7,
            conductivity: 0.16,
        },
        casing_contact: (0.36, 5.0),
        bounce: Bounce {
            restitution: 0.2,
            friction: 0.65,
            settle_speed: 55.0,
        },
        loose: VolcanicRock,
        toughness: 1050.0,
        blast_resistance: 340,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 0.0,
        melts: None,
    },
    // Mostly air: little to warm, and slow to pass heat on.
    // Snow
    MaterialDef {
        kind: Snow,
        phase: Phase::Powder { cohesive: false },
        thermal: Thermal {
            capacity: 1.2,
            conductivity: 0.05,
        },
        casing_contact: (0.1, 18.0),
        bounce: Bounce {
            restitution: 0.02,
            friction: 0.3,
            settle_speed: 140.0,
        },
        loose: Snow,
        toughness: 25.0,
        blast_resistance: 12,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 1.6,
        melts: None,
    },
    // Iron scrap: what is left of the Warden's plate, chains, pipes,
    // anvils, lantern frames. Loose (it falls and heaps, and is kicked
    // about), it takes and passes heat quickly, rings off a casing, rusts
    // in water and in acid, and melts only in the hottest fire.
    MaterialDef {
        kind: Metal,
        phase: Phase::Powder { cohesive: false },
        thermal: Thermal {
            capacity: 1.0,
            conductivity: 0.6,
        },
        casing_contact: (0.5, 3.0),
        bounce: Bounce {
            restitution: 0.35,
            friction: 0.5,
            settle_speed: 50.0,
        },
        loose: Metal,
        toughness: 2400.0,
        blast_resistance: 600,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 0.35,
        melts: Some((1500.0, Lava)),
    },
    // Rust: what iron crumbles into, a loose orange powder.
    MaterialDef {
        kind: Rust,
        phase: Phase::Powder { cohesive: false },
        thermal: Thermal {
            capacity: 1.2,
            conductivity: 0.2,
        },
        casing_contact: (0.1, 18.0),
        bounce: Bounce {
            restitution: 0.06,
            friction: 0.5,
            settle_speed: 120.0,
        },
        loose: Rust,
        toughness: 40.0,
        blast_resistance: 8,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 1.0,
        melts: None,
    },
    // Glass shards: broken crystals and panes, sand that lava touched.
    // Loose, acid-proof, it melts back into the flow.
    MaterialDef {
        kind: Glass,
        phase: Phase::Powder { cohesive: false },
        thermal: Thermal {
            capacity: 1.0,
            conductivity: 0.1,
        },
        casing_contact: (0.45, 4.0),
        bounce: Bounce {
            restitution: 0.3,
            friction: 0.4,
            settle_speed: 60.0,
        },
        loose: Glass,
        toughness: 120.0,
        blast_resistance: 30,
        ignition: NEVER,
        lifetime: 0,
        corrodes: 0.0,
        melts: Some((950.0, Lava)),
    },
    // Bone: skulls and scattered bones. It heaps loose, chars to ash in
    // a hot fire, and acid eats it.
    MaterialDef {
        kind: Bone,
        phase: Phase::Powder { cohesive: false },
        thermal: Thermal {
            capacity: 1.4,
            conductivity: 0.1,
        },
        casing_contact: (0.2, 10.0),
        bounce: Bounce {
            restitution: 0.18,
            friction: 0.5,
            settle_speed: 90.0,
        },
        loose: Bone,
        toughness: 160.0,
        blast_resistance: 30,
        ignition: 600.0,
        lifetime: 120,
        corrodes: 0.9,
        melts: None,
    },
    // Fungus: the caves' mushrooms, soft and wet. It burns once dried by
    // the heat, acid eats it fast, and where it sits in water it grows.
    MaterialDef {
        kind: Fungus,
        phase: Phase::Powder { cohesive: true },
        thermal: Thermal {
            capacity: 2.2,
            conductivity: 0.08,
        },
        casing_contact: (0.05, 16.0),
        bounce: Bounce {
            restitution: 0.04,
            friction: 0.35,
            settle_speed: 150.0,
        },
        loose: Fungus,
        toughness: 50.0,
        blast_resistance: 12,
        ignition: 230.0,
        lifetime: 140,
        corrodes: 1.4,
        melts: None,
    },
    // Torn silk (cobweb, dry stalks): light and loose, it goes up in a
    // flash at the least heat.
    MaterialDef {
        kind: Web,
        phase: Phase::Powder { cohesive: false },
        thermal: Thermal {
            capacity: 0.4,
            conductivity: 0.05,
        },
        casing_contact: (0.02, 30.0),
        bounce: Bounce {
            restitution: 0.0,
            friction: 0.2,
            settle_speed: 160.0,
        },
        loose: Web,
        toughness: 8.0,
        blast_resistance: 1,
        ignition: 120.0,
        lifetime: 6,
        corrodes: 2.5,
        melts: None,
    },
];
