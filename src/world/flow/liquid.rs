//! A liquid cell's step: falling, spreading along the surface, sinking
//! through lighter liquids, and squeezing out from under pressure.

use super::*;

impl LevelGrid {
    /// One liquid cell per tick, Noita style: fall (accelerating), slide down a
    /// diagonal, spill sideways under the weight of the liquid above, or glide
    /// along the surface toward somewhere lower. Every move is a real cell move.
    /// Returns where the cell ended up.
    pub(in crate::world) fn step_liquid(
        &mut self,
        mut cell: GridPosition,
        kind: TerrainKind,
        player: Option<Vec2>,
    ) -> GridPosition {
        let style = kind.liquid_style();
        let mut i = self.index(cell).unwrap();
        let above = GridPosition {
            x: cell.x,
            y: cell.y + 1,
        };
        let below = GridPosition {
            x: cell.x,
            y: cell.y - 1,
        };
        if self.get(above).is_some_and(TerrainKind::is_powder)
            && self.fluid_destination(cell, above, kind, player)
        {
            // Sinking powder waits on its dice; stay awake until it goes.
            self.keep_awake(cell);
            if self.roll(cell, 1) % 4 < SINK_CHANCE_IN_FOUR {
                self.swap_cells(cell, above);
                return above;
            }
        }
        if kind.is_aqueous()
            && self.roll(cell, 48) % 1000 < DIFFUSION
            && let Some(to) = self.diffuse(cell, kind)
        {
            return to;
        }
        if let Some(other) = self
            .get(below)
            .filter(|other| other.is_liquid() && *other != kind)
            && !self.grain_at(below)
            && !self.body_at(below)
        {
            // Heavier on lighter sinks through it, as fast as the density
            // difference allows: water drops through oil, blood settles
            // slowly through water.
            let chance = kind.sink_chance(other);
            if chance >= 1.0 || ((self.roll(cell, 47) % 1000) as f32) < chance * 1000.0 {
                self.swap_cells(cell, below);
                return below;
            }
            if chance > 0.0 {
                self.keep_awake(cell);
            }
        }
        // Resting liquid under a head of more liquid is pushed into any void
        // beside it at every depth, so an opening drains the body around it
        // instead of leaving a shaft cut through it.
        if self.flow[i].y > -1.5
            && let Some(to) = self.squeeze(cell, kind, player)
        {
            return to;
        }
        // Impulses can lift liquid; gravity then pulls the same conserved cells back.
        // The pull varies from column to column and moment to moment, so a
        // falling body shears into streaks that each hold together.
        // (A lone drop keeps plain gravity.)
        let gust = if self.get(above).is_some_and(TerrainKind::is_liquid) {
            // Mostly broad, slow undulation across the body, with a little
            // column-to-column grain.
            let broad = crate::world::materials::value_noise(
                cell.x as f32 / 5.0,
                self.tick as f32 / 40.0,
                53,
            ) * 2.0
                - 1.0;
            let grain = (crate::world::materials::noise(cell.x ^ 0x2f1d, (self.tick / 24) as i32)
                % 1000) as f32
                / 500.0
                - 1.0;
            1.0 + (broad * 0.8 + grain * 0.2) * FALL_SCATTER
        } else {
            1.0
        };
        let falling = self.flow[i].y - FALL_ACCELERATION * gust;
        self.flow[i].y = (falling / (1.0 + style.air_drag + style.air_drag_speed * falling.abs()))
            .clamp(-MAX_CELL_SPEED, MAX_CELL_SPEED);
        let vertical = if self.flow[i].y > 0.0 { 1 } else { -1 };
        if self.open_for_liquid(GridPosition {
            x: cell.x,
            y: cell.y + vertical,
        }) {
            // Accumulate signed subcell travel, including the apex of a splash.
            // Forcing a cell of motion every tick made slow water jump and jitter.
            self.phase[i].y += self.flow[i].y;
            // But liquid right behind a pixel that just moved out from under
            // it goes with it at once: a draining column or a stream stays
            // one body instead of peeling off row by row.
            if vertical < 0
                && self
                    .index(below)
                    .is_some_and(|j| self.updated[j] == self.stamp())
            {
                self.flow[i].y = self.flow[i].y.min(-0.9);
                self.phase[i].y = self.phase[i].y.min(-1.0);
            }
            let distance = (self.phase[i].y.abs() as i32).min(MAX_CELL_SPEED as i32);
            self.phase[i].y -= vertical as f32 * distance as f32;
            for _ in 0..distance {
                let next = GridPosition {
                    x: cell.x,
                    y: cell.y + vertical,
                };
                if !self.open_for_liquid(next) {
                    break;
                }
                self.swap_cells(cell, next);
                cell = next;
                i = self.index(cell).unwrap();
            }
            if vertical < 0
                && let Some(to) = self.break_up(cell, kind)
            {
                return to;
            }
            // Air slows a stream in both axes. It can still carry a sideways
            // impulse, but a drop cannot coast across the whole cave.
            let side_speed = self.flow[i].x.abs();
            self.flow[i].x /= 1.0 + style.air_drag + style.air_drag_speed * side_speed;
            self.advect_liquid(&mut cell, kind, player, false);
            return cell;
        }
        if vertical < 0
            && let Some(j) = self.index(below)
            && self.cells[j].is_some_and(TerrainKind::is_liquid)
            && self.flow[j].y < -0.05
        {
            // Catching up with liquid that is itself still falling: follow it
            // down instead of landing on it and fanning out in mid-air, in
            // step with it, so the two move together once it has room.
            self.flow[i].y = self.flow[i].y.max(self.flow[j].y);
            self.phase[i].y = self.phase[j].y;
            let side_speed = self.flow[i].x.abs();
            self.flow[i].x /= 1.0 + style.air_drag + style.air_drag_speed * side_speed;
            self.advect_liquid(&mut cell, kind, player, false);
            return cell;
        }
        let impact = -self.flow[i].y;
        if impact > 0.9 {
            self.hear(super::soundscape::Noise::Flow, cell, impact);
        }
        if impact > SPLASH_IMPACT && self.splash_back(cell, impact) {
            self.hear(super::soundscape::Noise::Splash, cell, impact);
            return cell;
        }
        // A thin layer wets a solid ledge. Capillary hold keeps slow drips on
        // the ledge instead of routing every isolated pixel straight to its
        // nearest edge. A deeper column, or a hard impact, can still spill.
        let held_film = self.get(below).is_some_and(TerrainKind::is_solid)
            && !self.get(above).is_some_and(TerrainKind::is_liquid)
            && impact < 1.5;
        let side = self.preferred_side(cell, self.flow[i].x);
        if impact > 1.2 {
            // A falling jet fans out sideways where it lands.
            self.flow[i].x = side as f32 * (self.flow[i].x.abs() + impact * 0.3).min(2.5);
        }
        self.flow[i].y = 0.0;
        self.phase[i].y = 0.0;
        for side in [side, -side] {
            let beside = GridPosition {
                x: cell.x + side,
                y: cell.y,
            };
            let diagonal = GridPosition {
                x: cell.x + side,
                y: cell.y - 1,
            };
            // Slide off slopes and edges; never squeeze through a sealed corner.
            if !held_film
                && self.fluid_destination(cell, beside, kind, player)
                && self.fluid_destination(cell, diagonal, kind, player)
            {
                self.swap_cells(cell, diagonal);
                let j = self.index(diagonal).unwrap();
                self.flow[j].x = side as f32 * self.flow[j].x.abs().max(style.spread * 0.5);
                return diagonal;
            }
        }
        if self.get(above).is_some_and(TerrainKind::is_liquid) {
            // Liquid above pushes this cell out through any side opening. The
            // deeper the head, the faster the spill, so breaches gush and a
            // released column collapses from its base.
            let open = [side, -side].into_iter().find(|&side| {
                self.fluid_destination(
                    cell,
                    GridPosition {
                        x: cell.x + side,
                        y: cell.y,
                    },
                    kind,
                    player,
                )
            });
            if let Some(side) = open {
                let push = (style.spread + (self.liquid_load(cell) as f32).sqrt() * style.head)
                    .min(MAX_CELL_SPEED);
                if self.flow[i].x * (side as f32) < push {
                    self.flow[i].x = side as f32 * push;
                }
            } else {
                self.flow[i].x *= 0.5;
            }
        } else if let Some(side) = (!held_film)
            .then(|| self.surface_route(cell, kind, side, player))
            .flatten()
        {
            self.flow[i].x += (side as f32 * style.spread - self.flow[i].x) * 0.5;
        } else {
            self.flow[i].x *= style.damping;
            if self.flow[i].x.abs() < 0.1
                && let Some(to) = self.wander(cell, kind)
            {
                return to;
            }
        }
        self.advect_liquid(&mut cell, kind, player, true);
        cell
    }

    /// A resting pixel with liquid piled on it and a void beside it (open,
    /// with nothing under it either) is pushed out into the void, more
    /// likely the deeper it sits. Returns where it went.
    pub(super) fn squeeze(
        &mut self,
        cell: GridPosition,
        kind: TerrainKind,
        player: Option<Vec2>,
    ) -> Option<GridPosition> {
        let above = GridPosition {
            x: cell.x,
            y: cell.y + 1,
        };
        if !self.get(above).is_some_and(TerrainKind::is_liquid) {
            return None;
        }
        let void = |grid: &Self, side: i32| {
            let beside = GridPosition {
                x: cell.x + side,
                y: cell.y,
            };
            let under = GridPosition {
                x: beside.x,
                y: cell.y - 1,
            };
            (grid.open_for_liquid(beside)
                && grid.open_for_liquid(under)
                && grid.pressed(cell, side)
                && !grid.blocked_by_player(beside, player))
            .then_some(beside)
        };
        let dice = self.roll(cell, 50);
        let first = if dice & 1 == 0 { 1 } else { -1 };
        let (side, beside) = [first, -first]
            .into_iter()
            .find_map(|side| void(self, side).map(|beside| (side, beside)))?;
        let style = kind.liquid_style();
        let load = self.liquid_load(cell);
        let chance = load as f32 / (load + SQUEEZE_DEPTH) as f32 * (style.spread / 2.0).min(1.0);
        if ((dice >> 4) % 1000) as f32 >= chance * 1000.0 {
            // It will go on a later tick.
            self.keep_awake(cell);
            return None;
        }
        self.swap_cells(cell, beside);
        let j = self.index(beside).unwrap();
        let push = (style.spread + (load as f32).sqrt() * style.head).min(MAX_CELL_SPEED);
        self.flow[j] = Vec2::new(side as f32 * push * 0.5, -0.6);
        Some(beside)
    }

    /// Whether the liquid under the open cell `gap` is running away down:
    /// open below it, or liquid falling.
    pub(super) fn draining_under(&self, gap: GridPosition) -> bool {
        let under = GridPosition {
            x: gap.x,
            y: gap.y - 1,
        };
        self.open_for_liquid(under)
            || self.index(under).is_some_and(|j| {
                self.cells[j].is_some_and(TerrainKind::is_liquid) && self.flow[j].y < -0.3
            })
    }

    /// Whether this pixel is part of a body of liquid bearing the weight of
    /// more on top: it rests on liquid or ground, has liquid or ground beside
    /// it on the side away from `side`, and its column stands on the ground.
    /// A strand of a falling stream, or a sheet in midair, is under no
    /// pressure.
    pub(super) fn pressed(&self, cell: GridPosition, side: i32) -> bool {
        let held = |dx: i32, dy: i32| {
            self.get(GridPosition {
                x: cell.x + dx,
                y: cell.y + dy,
            })
            .is_some_and(|kind| !kind.is_gas())
        };
        if !held(0, -1) || !held(-side, 0) {
            return false;
        }
        // And the liquid under it stands on something, down its column
        // through liquid and the odd gap a draining column opens: a sheet
        // or jet in midair presses on nothing.
        let mut gaps = 0;
        for dy in 1..=40 {
            match self.get(GridPosition {
                x: cell.x,
                y: cell.y - dy,
            }) {
                Some(kind) if kind.is_liquid() => {}
                Some(kind) if kind.is_solid() => return true,
                _ if cell.y - dy < 0 => return true,
                _ => {
                    gaps += 1;
                    if gaps > 2 {
                        return false;
                    }
                }
            }
        }
        true
    }
}
