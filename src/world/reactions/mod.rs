//! Chemistry that runs whether or not anything is hot, driven by the
//! reaction table in `materials.rs` (`REACTIONS`): acid eating what it
//! touches, including water and ice, and fuming away into the air; blood stains
//! solids it touches, which water rinses out again.
//!
//! The two-cell rule conserves every liquid and solid pixel in this game:
//! acid and what it eats become flammable fumes. Gases alone can thin into
//! air. Noita has additional reaction types, so this is a game constraint.
//!
//! The cells involved (reagents, blood, stained solids) are tracked in
//! `Activity::reactive`, so a still pool of acid in a sleeping chunk keeps
//! eating its basin.

use super::*;
use crate::world::materials::{MAX_STAIN, Product, Reaction, ReactionEffect};

/// Neighbors ordinary reagents try, one per tick, weighted toward the floor
/// they rest on. Lava checks every cardinal contact below.
const REACH: [(i32, i32); 9] = [
    (0, -1),
    (0, -1),
    (-1, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (0, 1),
    (-1, 1),
    (1, 1),
];
/// Stain a touching blood pixel adds per soak, and its chance per mille.
const STAIN_STEP: u8 = 26;
const STAIN_CHANCE: u32 = 220;
/// Stain water washes out per rinse, and its chance per mille (three
/// times as likely where the water moves).
const WASH_STEP: u8 = 10;
const WASH_CHANCE: u32 = 110;

impl LevelGrid {
    pub fn step_reactions(&mut self) {
        if self.active.reaction_front.is_empty() {
            return;
        }
        // Cells made by a reaction this tick do not react again until the
        // next one; the flood-fill stamps mark them.
        self.visit_stamp = self.visit_stamp.wrapping_add(1);
        if self.visit_stamp == 0 {
            self.visited.fill(0);
            self.visit_stamp = 1;
        }
        let mut cells = std::mem::take(&mut self.active.reaction_work);
        cells.clear();
        self.active
            .reaction_front
            .gather(&self.chunks, self.sleep.live(), &mut cells);
        for &i in &cells {
            if !self.active.reaction_front.contains(i) {
                continue;
            }
            let cell = self.cell_of(i);
            let Some(kind) = self.cells[i] else { continue };
            if kind == TerrainKind::Blood {
                self.soak_blood(cell);
            } else if kind.is_solid() && self.stain[i] > 0 {
                self.rinse(cell, i);
            }
            if self.cells[i] == Some(kind) && self.visited[i] != self.visit_stamp {
                self.react(cell, kind);
            }
        }
        self.active.reaction_work = cells;
    }

    /// Try touching materials against the reagent's reactions. Ordinary
    /// reagents sample one neighbor; lava rolls once per cardinal contact.
    /// At most one reaction happens per reagent cell and tick.
    fn react(&mut self, cell: GridPosition, kind: TerrainKind) {
        if kind == TerrainKind::Lava {
            // The table's quench odds are per *contact*. Picking one of nine
            // directions first made a broad water/lava interface react at
            // less than a tenth of its intended rate, leaving water perched
            // in a solid-looking slab over a still molten pool.
            let start = (self.roll(cell, 60) % 4) as usize;
            let neighbors = [(0, 1), (-1, 0), (1, 0), (0, -1)];
            for n in 0..4 {
                let (dx, dy) = neighbors[(start + n) % 4];
                let target = GridPosition {
                    x: cell.x + dx,
                    y: cell.y + dy,
                };
                let Some(j) = self.index(target) else {
                    continue;
                };
                if self.grain_at(target) || self.visited[j] == self.visit_stamp {
                    continue;
                }
                let partner = self.cells[j];
                if let Some(reaction) = kind.reactions().find(|row| {
                    row.partner.affinity(partner) > 0.0
                        && self.roll(target, 61 + n as u32) % 1000 < row.per_mille as u32
                }) {
                    self.apply_reaction(reaction, cell, target);
                    return;
                }
            }
            return;
        }
        let dice = self.roll(cell, 61);
        let (dx, dy) = REACH[(dice % REACH.len() as u32) as usize];
        let target = GridPosition {
            x: cell.x + dx,
            y: cell.y + dy,
        };
        let Some(j) = self.index(target) else { return };
        if self.grain_at(target) || self.visited[j] == self.visit_stamp {
            return;
        }
        let partner = self.cells[j];
        let odds = (dice >> 8) % 1000;
        let mut threshold = 0.0;
        for reaction in kind.reactions() {
            let affinity = reaction.partner.affinity(partner);
            if affinity == 0.0 {
                continue;
            }
            threshold += reaction.per_mille as f32 * affinity;
            if (odds as f32) < threshold {
                self.apply_reaction(reaction, cell, target);
                return;
            }
        }
    }

    fn apply_reaction(&mut self, reaction: &Reaction, cell: GridPosition, target: GridPosition) {
        let original_partner = self.get(target);
        for (at, product) in [
            (cell, reaction.reagent_into),
            (target, reaction.partner_into),
        ] {
            match product {
                Product::Same => {}
                Product::Air => self.put(at, None),
                Product::Into(kind) => self.transmute(at, kind),
            }
            if reaction.heat != 0.0
                && let Some(i) = self.index(at)
                && self.cells[i].is_some()
            {
                self.temperature[i] = (self.temperature[i] + reaction.heat).clamp(-30.0, 1400.0);
                self.track_heat(i);
            }
            let i = self.index(at).unwrap();
            // Quenching consumes the molten pixel's heat. Its new rock can
            // warm up and melt again later, but cannot instantly undo the
            // water reaction on the next chemistry tick.
            if reaction.reagent == TerrainKind::Lava
                && matches!(
                    reaction.partner,
                    crate::world::materials::reactions::Partner::Kind(
                        TerrainKind::Water | TerrainKind::Blood
                    )
                )
            {
                self.temperature[i] = if at == cell { 180.0 } else { 130.0 };
                self.track_heat(i);
            }
            if self.cells[i] == Some(TerrainKind::Fire) {
                self.temperature[i] = 720.0;
                self.track_heat(i);
            }
            self.visited[i] = self.visit_stamp;
        }
        if let Some(effect) = reaction.effect {
            self.reaction_effect(cell, target, original_partner, effect);
        }
        self.dirty = true;
    }

    /// Light, froth and bubbles where a reaction happened. `cell` is the
    /// reagent, `target` what it reacted with (both already transformed).
    fn reaction_effect(
        &mut self,
        cell: GridPosition,
        target: GridPosition,
        original_partner: Option<TerrainKind>,
        effect: ReactionEffect,
    ) {
        let dice = self.roll(target, 69);
        let eaten = original_partner.is_some_and(|kind| !kind.is_gas());
        if original_partner.is_some_and(TerrainKind::is_liquid) {
            self.hear(super::soundscape::Noise::Mix, target, 1.0);
        } else if eaten {
            self.hear(super::soundscape::Noise::Corrode, target, 1.0);
        }
        // The front sizzles where it bites: a flickering light on what is
        // being eaten, froth in the liquid doing the eating.
        if eaten {
            let at = self
                .cell_to_world(target)
                .lerp(self.cell_to_world(cell), 0.35);
            self.sizzle(at, effect.color, 14.0 + (dice % 22) as f32);
        }
        // Liquid eating something fizzes: bubbles stream up out of the
        // contact even when what it ate was solid, while the real gas the
        // reaction made rises beside them.
        if effect.bubbles
            && eaten
            && self.get(cell).is_some_and(TerrainKind::is_liquid)
            && !original_partner.is_some_and(TerrainKind::is_liquid)
        {
            for k in 0..1 + (dice >> 8) % 2 {
                let jitter = Vec2::new(((dice >> (12 + k * 4)) % 5) as f32 - 2.0, 0.0);
                self.reaction_bubble(self.cell_to_world(cell) + jitter, effect.color);
            }
        }
        // At an open surface, the bite spits a spark or two into the air.
        let open_above = |grid: &Self, at: GridPosition| {
            grid.get(GridPosition {
                x: at.x,
                y: at.y + 1,
            })
            .is_none_or(TerrainKind::is_gas)
        };
        if eaten && (open_above(self, cell) || open_above(self, target)) && (dice >> 16) % 5 < 3 {
            self.reaction_glint(target, effect.color);
        }
        if original_partner.is_some_and(TerrainKind::is_liquid) {
            let mut bubble_site = None;
            for (dx, dy) in [(0, -1), (-1, 0), (1, 0), (0, 1)] {
                let near = GridPosition {
                    x: target.x + dx,
                    y: target.y + dy,
                };
                let Some(i) = self.index(near) else { continue };
                if !self.cells[i].is_some_and(TerrainKind::is_liquid) {
                    continue;
                }
                bubble_site.get_or_insert(near);
                self.flow[i] += Vec2::new(dx as f32 * 0.5, 0.8) * effect.agitation;
                self.flow[i] =
                    self.flow[i].clamp(Vec2::splat(-MAX_CELL_SPEED), Vec2::splat(MAX_CELL_SPEED));
                self.wake(near);
            }
            if effect.bubbles
                && self.roll(target, 66).is_multiple_of(2)
                && let Some(near) = bubble_site
            {
                self.reaction_bubble(self.cell_to_world(near), effect.color);
            }
        }
        if !eaten && self.roll(target, 64).is_multiple_of(4) {
            self.reaction_glint(target, effect.color);
        }
        // A violent reaction at an open surface spits the reagent out of
        // it: the pixel itself flies, so nothing is made from nothing.
        if effect.spit > 0.0
            && ((dice >> 20) % 1000) as f32 / 1000.0 < effect.spit
            && open_above(self, cell)
            && let Some(kind) = self.get(cell).filter(|kind| kind.is_liquid())
        {
            let point = self.cell_to_world(cell);
            let side = ((dice >> 4) % 200) as f32 - 100.0;
            let up = 90.0 + ((dice >> 12) % 160) as f32;
            self.put(cell, None);
            if !self.launch_liquid(point, Vec2::new(side, up), kind, 3) {
                self.put(cell, Some(kind));
            }
            self.reaction_glint(cell, effect.color);
        }
    }

    /// Turn the cell at `at` into `kind` in place: it keeps its heat and some
    /// of its motion, and takes on the new material's look and lifetime.
    pub(super) fn transmute(&mut self, at: GridPosition, kind: TerrainKind) {
        let Some(i) = self.index(at) else { return };
        let temperature = self.temperature[i];
        self.change_phase(at, kind, temperature);
    }

    /// Blood soaks into one touching solid, by chance.
    fn soak_blood(&mut self, cell: GridPosition) {
        let dice = self.roll(cell, 62);
        if dice % 1000 >= STAIN_CHANCE {
            return;
        }
        let (dx, dy) =
            [(0, -1), (-1, 0), (1, 0), (0, 1), (-1, -1), (1, -1)][((dice >> 10) % 6) as usize];
        let target = GridPosition {
            x: cell.x + dx,
            y: cell.y + dy,
        };
        let Some(j) = self.index(target) else { return };
        if !self.cells[j].is_some_and(TerrainKind::is_solid) || self.stain[j] >= MAX_STAIN {
            return;
        }
        self.stain[j] = self.stain[j].saturating_add(STAIN_STEP).min(MAX_STAIN);
        self.active.reactive.set(j, true);
        self.refresh_reaction_front(j);
        self.render_cache.mark_color(&self.chunks, target);
        self.dirty = true;
    }

    /// Water (or acid) touching a stained solid rinses the stain out a
    /// little at a time, faster where the water moves.
    fn rinse(&mut self, cell: GridPosition, i: usize) {
        let dice = self.roll(cell, 63);
        let next = [(0, 1), (-1, 0), (1, 0), (0, -1)][(dice % 4) as usize];
        let next = GridPosition {
            x: cell.x + next.0,
            y: cell.y + next.1,
        };
        let Some(j) = self.index(next) else { return };
        if !matches!(self.cells[j], Some(TerrainKind::Water | TerrainKind::Acid)) {
            return;
        }
        let chance = if self.flow[j] == Vec2::ZERO {
            WASH_CHANCE
        } else {
            WASH_CHANCE * 3
        };
        if (dice >> 8) % 1000 >= chance {
            return;
        }
        self.stain[i] = self.stain[i].saturating_sub(WASH_STEP);
        if self.stain[i] == 0 {
            self.active.reactive.set(i, false);
        }
        self.refresh_reaction_front(i);
        self.render_cache.mark_color(&self.chunks, cell);
        self.dirty = true;
    }

    /// How much blood a solid holds (0..=255), for anything that wants to show it.
    #[cfg(test)]
    pub fn stain_at(&self, cell: GridPosition) -> u8 {
        self.index(cell).map_or(0, |i| self.stain[i])
    }
}
