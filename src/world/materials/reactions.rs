//! The table of chemical reactions: which materials react with which, and
//! what comes of it.

use super::*;

/// A material family a reaction can target. New materials opt in here instead
/// of requiring another copy of every reaction that acts on the family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialTag {
    Corrodible,
}

/// What a reaction meets beside its reagent: a material, a material family,
/// or open air (an empty cell). Smoke and steam are distinct gases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Partner {
    Kind(TerrainKind),
    Tag(MaterialTag),
    Air,
}

impl Partner {
    pub fn affinity(self, cell: Option<TerrainKind>) -> f32 {
        match self {
            Self::Kind(kind) => f32::from(cell == Some(kind)),
            Self::Tag(tag) => cell.map_or(0.0, |kind| kind.reaction_affinity(tag)),
            Self::Air => f32::from(cell.is_none()),
        }
    }
}

/// What one side of a reaction turns into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Product {
    /// Unchanged.
    Same,
    Into(TerrainKind),
    /// Gone into the air. Only a gas may do this: liquid and solid pixels
    /// always become something (see `reactions_conserve_every_liquid_and_solid_pixel`).
    /// Acid touching fumes uses this to clear the contacted gas cell.
    Air,
}

/// Visual and motion cues emitted by a successful contact reaction.
#[derive(Clone, Copy, Debug)]
pub struct ReactionEffect {
    pub color: [u8; 3],
    /// Motion handed to neighboring liquid at the contact.
    pub agitation: f32,
    /// Cosmetic bubbles rise through neighboring liquid. The gas product
    /// remains the physical material made by the reaction.
    pub bubbles: bool,
    /// Chance that a reagent at an open surface is thrown up out of it:
    /// the reaction spits. The pixel is moved, never made.
    pub spit: f32,
}

/// One material reaction, Noita style: two touching cells, `reagent` and
/// `partner`, turn into two cells. A pixel of liquid or solid only ever
/// becomes another material; a gas may clear into air. When more than one
/// row matches a contact, their chances add up and at most one happens.
#[derive(Clone, Copy, Debug)]
pub struct Reaction {
    pub reagent: TerrainKind,
    pub partner: Partner,
    /// Chance per mille per tick, each time the reagent tries this contact.
    pub per_mille: u16,
    pub reagent_into: Product,
    pub partner_into: Product,
    /// Degrees added to both cells (negative absorbs heat).
    pub heat: f32,
    /// Contact light and fluid wake, separate from the material products.
    pub effect: Option<ReactionEffect>,
}

pub(super) const fn reaction(
    reagent: TerrainKind,
    partner: Partner,
    per_mille: u16,
    reagent_into: Product,
    partner_into: Product,
) -> Reaction {
    Reaction {
        reagent,
        partner,
        per_mille,
        reagent_into,
        partner_into,
        heat: 0.0,
        effect: None,
    }
}

pub(super) const fn with_effect(mut reaction: Reaction, effect: ReactionEffect) -> Reaction {
    reaction.effect = Some(effect);
    reaction
}

/// Every reaction in the world. Heat-driven changes (burning, boiling,
/// melting, condensing) live with the heat model in `world/chemistry/`;
/// these are the ones that happen at any temperature.
pub const REACTIONS: [Reaction; 27] = {
    use Product::*;
    use TerrainKind::*;
    [
        // Acid meeting water is violent: it hisses and heats, foams, and
        // spits drops out of the surface, while the water it takes becomes
        // flammable gas. (Noita: acid + water -> flammable gas.)
        with_effect(
            Reaction {
                heat: 14.0,
                ..reaction(Acid, Partner::Kind(Water), 70, Same, Into(Fumes))
            },
            ReactionEffect {
                color: [206, 255, 150],
                agitation: 1.1,
                bubbles: true,
                spit: 0.3,
            },
        ),
        // Flesh dissolves fast: it foams pink and melts, part of it into
        // fumes and part into the blood that was in it.
        with_effect(
            reaction(Acid, Partner::Kind(Flesh), 200, Same, Into(Fumes)),
            ReactionEffect {
                color: [255, 150, 120],
                agitation: 0.5,
                bubbles: true,
                spit: 0.08,
            },
        ),
        with_effect(
            reaction(Acid, Partner::Kind(Flesh), 80, Same, Into(Blood)),
            ReactionEffect {
                color: [255, 110, 110],
                agitation: 0.4,
                bubbles: true,
                spit: 0.0,
            },
        ),
        // Noita's corrodible tag includes water and ice. The affinity above
        // slows cave rock while keeping an acid-water contact lively.
        with_effect(
            reaction(
                Acid,
                Partner::Tag(MaterialTag::Corrodible),
                50,
                Same,
                Into(Fumes),
            ),
            ReactionEffect {
                color: [150, 245, 96],
                agitation: 0.28,
                bubbles: true,
                spit: 0.0,
            },
        ),
        // A small chance to spend the acid too keeps one isolated drop from
        // indefinitely excavating a whole cave.
        with_effect(
            reaction(
                Acid,
                Partner::Tag(MaterialTag::Corrodible),
                6,
                Into(Fumes),
                Into(Fumes),
            ),
            ReactionEffect {
                color: [190, 255, 116],
                agitation: 0.35,
                bubbles: true,
                spit: 0.0,
            },
        ),
        // Acid touching its own flammable gas becomes gas and clears the
        // contacted gas cell, like Noita's acid + flammable gas reaction.
        with_effect(
            reaction(Acid, Partner::Kind(Fumes), 12, Into(Fumes), Air),
            ReactionEffect {
                color: [224, 255, 146],
                agitation: 0.0,
                bubbles: false,
                spit: 0.0,
            },
        ),
        with_effect(
            reaction(Acid, Partner::Air, 10, Into(Fumes), Same),
            ReactionEffect {
                color: [160, 235, 92],
                agitation: 0.0,
                bubbles: false,
                spit: 0.0,
            },
        ),
        // Noita's lava reactions: each contact turns both pixels into real
        // products. Blood makes a dark, heat-resistant volcanic crust.
        with_effect(
            reaction(Lava, Partner::Kind(Water), 800, Into(Stone), Into(Steam)),
            ReactionEffect {
                color: [255, 221, 116],
                agitation: 1.2,
                bubbles: true,
                spit: 0.0,
            },
        ),
        with_effect(
            reaction(
                Lava,
                Partner::Kind(Blood),
                700,
                Into(VolcanicRock),
                Into(Steam),
            ),
            ReactionEffect {
                color: [255, 144, 98],
                agitation: 1.0,
                bubbles: true,
                spit: 0.0,
            },
        ),
        // Snow on lava flashes to steam where it touches.
        with_effect(
            reaction(Lava, Partner::Kind(Snow), 900, Same, Into(Steam)),
            ReactionEffect {
                color: [255, 236, 200],
                agitation: 0.4,
                bubbles: false,
                spit: 0.0,
            },
        ),
        // Water soaking into snow freezes with it into ice: a drift a pool
        // runs into sets hard, and a slush of the two turns glassy. It
        // takes a little heat out of what is round it.
        with_effect(
            Reaction {
                heat: -3.0,
                ..reaction(Snow, Partner::Kind(Water), 40, Into(Ice), Into(Ice))
            },
            ReactionEffect {
                color: [214, 236, 255],
                agitation: 0.05,
                bubbles: false,
                spit: 0.0,
            },
        ),
        reaction(Lava, Partner::Kind(Oil), 80, Same, Into(Fire)),
        reaction(Lava, Partner::Kind(Wood), 80, Same, Into(Fire)),
        reaction(Lava, Partner::Kind(Coal), 80, Same, Into(Fire)),
        reaction(Lava, Partner::Kind(Flesh), 80, Same, Into(Fire)),
        reaction(Lava, Partner::Kind(Fungus), 90, Same, Into(Fire)),
        reaction(Lava, Partner::Kind(Web), 400, Same, Into(Fire)),
        reaction(Lava, Partner::Kind(Bone), 25, Same, Into(Ash)),
        // Noita's lava melts sand into glass where it runs over it.
        with_effect(
            reaction(Lava, Partner::Kind(Sand), 35, Same, Into(Glass)),
            ReactionEffect {
                color: [255, 214, 150],
                agitation: 0.2,
                bubbles: false,
                spit: 0.0,
            },
        ),
        // Ice flashes as water does, leaving a crust of rock.
        with_effect(
            reaction(Lava, Partner::Kind(Ice), 700, Into(Stone), Into(Steam)),
            ReactionEffect {
                color: [255, 232, 180],
                agitation: 1.0,
                bubbles: true,
                spit: 0.0,
            },
        ),
        // It slowly takes loose rubble into itself.
        reaction(Lava, Partner::Kind(Rubble), 3, Same, Into(Lava)),
        // Acid on iron: a fizz of orange as it crumbles to rust.
        with_effect(
            reaction(Acid, Partner::Kind(Metal), 30, Same, Into(Rust)),
            ReactionEffect {
                color: [255, 170, 90],
                agitation: 0.3,
                bubbles: true,
                spit: 0.02,
            },
        ),
        // Iron left in water rusts, slowly.
        reaction(Metal, Partner::Kind(Water), 1, Into(Rust), Same),
        reaction(Metal, Partner::Kind(Blood), 1, Into(Rust), Same),
        // Mushrooms drink water and grow into it.
        reaction(Fungus, Partner::Kind(Water), 2, Same, Into(Fungus)),
        // Bone and fungus foam away in acid.
        with_effect(
            reaction(Acid, Partner::Kind(Fungus), 160, Same, Into(Fumes)),
            ReactionEffect {
                color: [210, 150, 230],
                agitation: 0.4,
                bubbles: true,
                spit: 0.05,
            },
        ),
        // Warm blood melts the snow it soaks into.
        reaction(Blood, Partner::Kind(Snow), 15, Same, Into(Water)),
    ]
};

/// Built from the table at compile time, so routine cell swaps can refresh
/// activity in O(1) even as the number of reaction rows grows.
pub(super) const REAGENT_FLAGS: [bool; 256] = {
    let mut flags = [false; 256];
    let mut i = 0;
    while i < REACTIONS.len() {
        flags[REACTIONS[i].reagent as usize] = true;
        i += 1;
    }
    flags
};

impl TerrainKind {
    /// Relative corrosion speed, with zero meaning acid-proof. Noita marks
    /// water, blood, ice and many solids as corrodible; this game's slower
    /// stone and coal preserve the shape of a cave long enough to explore it.
    pub fn reaction_affinity(self, tag: MaterialTag) -> f32 {
        match tag {
            MaterialTag::Corrodible => self.def().corrodes,
        }
    }

    /// Reactions this material drives, in table order.
    pub fn reactions(self) -> impl Iterator<Item = &'static Reaction> {
        REACTIONS
            .iter()
            .filter(move |reaction| reaction.reagent == self)
    }
}
