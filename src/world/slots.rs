//! The memory behind the chunk slots: every per-cell array holds exactly
//! the chunk map's slots, with room kept ahead for as many as the live
//! window may need, and chunks are moved down into free slots so the arrays
//! can shrink after the resident set does.

use super::*;

impl LevelGrid {
    /// Make every per-cell (and per-texel, per-chunk) array hold exactly the
    /// chunk map's slots.
    pub(super) fn fit_to_capacity(&mut self) {
        use activity::fit_cells;
        let slots = self.chunks.capacity();
        macro_rules! fit {
            ($($array:ident: $fill:expr),* $(,)?) => {
                $(fit_cells(&mut self.$array, slots, $fill);)*
            };
        }
        fit!(
            cells: None,
            flow: Vec2::ZERO,
            phase: Vec2::ZERO,
            base_colors: [0; 4],
            carried: false,
            stain: 0,
            temperature: 20.0,
            thermal_delta: 0.0,
            damage: 0.0,
            life: 0,
            updated: 0,
            pressure: 0,
            visited: 0,
            airborne: None,
            airborne_count: 0,
        );
        self.active.fit(slots);
        self.sleep.fit(slots);
        self.render_cache.fit(slots);
    }

    /// Move the chunk in slot `from` to the free slot `to` in every per-slot
    /// array (the chunk map has already moved it).
    pub(super) fn move_slot(&mut self, from: usize, to: usize) {
        use activity::move_cells;
        macro_rules! move_all {
            ($($array:ident),* $(,)?) => {
                $(move_cells(&mut self.$array, from, to);)*
            };
        }
        move_all!(
            cells,
            flow,
            phase,
            base_colors,
            carried,
            stain,
            temperature,
            thermal_delta,
            damage,
            life,
            updated,
            pressure,
            visited,
            airborne,
            airborne_count,
        );
        self.active.relocate(from, to);
        self.sleep.relocate(from, to);
        self.render_cache.relocate(from, to);
    }

    /// Pack the resident chunks into the lowest slots, up to `budget` of them
    /// moved, and let the per-cell arrays shrink to what is left: memory
    /// follows what is resident, even after the whole level was (when it
    /// was generated) or a long fall left a trail behind.
    pub fn compact_slots(&mut self, budget: usize) {
        let before = self.chunks.capacity();
        let moves = self.chunks.compact(budget);
        for &(from, to) in &moves {
            self.move_slot(from, to);
        }
        if self.chunks.capacity() < before {
            self.fit_to_capacity();
        }
    }

    /// Keep room in every per-cell array for `slots` chunks (see
    /// `activity::keep_room`): reserved ahead, so chunks coming in never
    /// make the arrays move, and given back when far more is held.
    pub fn keep_room(&mut self, slots: usize) {
        use activity::keep_room;
        macro_rules! room {
            ($($array:ident),* $(,)?) => {
                $(keep_room(&mut self.$array, slots);)*
            };
        }
        room!(
            cells,
            flow,
            phase,
            base_colors,
            carried,
            stain,
            temperature,
            thermal_delta,
            damage,
            life,
            updated,
            pressure,
            visited,
            airborne,
            airborne_count,
        );
        self.active.keep_room(slots);
        self.render_cache.keep_room(slots);
    }
}
