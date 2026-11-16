//! Powders and grains: a powder cell's step (falling, sliding, piling,
//! climbing out when buried), and grains in flight, from launch to landing.

use super::*;

impl LevelGrid {
    pub(super) fn blocked_by_player(&self, pos: GridPosition, player: Option<Vec2>) -> bool {
        player.is_some_and(|player| {
            let delta = (self.cell_to_world(pos) - player).abs();
            let half = PLAYER_HALF_SIZE + Vec2::splat(self.pixel_size * 0.5 + 0.5);
            delta.x < half.x && delta.y < half.y
        })
    }

    pub(super) fn free_for_material(&self, pos: GridPosition, player: Option<Vec2>) -> bool {
        let Some(i) = self.index(pos) else {
            return false;
        };
        self.cells[i].is_none_or(TerrainKind::is_gas)
            && self.airborne[i].is_none()
            && !self.blocked_by_player(pos, player)
            && !self.body_at(pos)
    }

    pub(super) fn grain_at(&self, pos: GridPosition) -> bool {
        self.index(pos)
            .is_some_and(|index| self.airborne[index].is_some())
    }

    /// Reserve (Some) or release (None) one grain's claim on the cell at `position`.
    pub(super) fn reserve_grain(&mut self, position: Vec2, occupied: Option<TerrainKind>) {
        let cell = self.world_to_cell(position);
        // A grain holds its cell against powder and liquid; whatever it
        // blocked may move again once it has passed.
        self.keep_awake(cell);
        if let Some(index) = self.index(cell) {
            let count = &mut self.airborne_count[index];
            if occupied.is_some() {
                *count = count.saturating_add(1);
                self.airborne[index] = occupied;
            } else {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.airborne[index] = None;
                }
            }
        }
    }

    pub fn step_powders(&mut self, player: Option<Vec2>) {
        // Bottom-up means a cell moves at most once per tick. Falling powder
        // accelerates like everything else; resting powder slides off slopes
        // toward a randomly chosen side so piles do not lean. Only chunks
        // where something changed are visited: a settled bank sleeps.
        self.tick += 1;
        self.update_live(player);
        self.wake_movers(player);
        self.for_awake_cells(true, sleep::AwakeMaterial::Powder, |grid, cell, index| {
            if cell.y > 0 && grid.cells[index].is_some_and(TerrainKind::is_powder) {
                grid.step_powder(cell, index, player);
            }
        });
    }

    pub(super) fn step_powder(&mut self, cell: GridPosition, index: usize, player: Option<Vec2>) {
        let GridPosition { x, y } = cell;
        let Some(kind) = self.get(cell).filter(|kind| kind.is_powder()) else {
            return;
        };
        let below = GridPosition { x, y: y - 1 };
        if kind == TerrainKind::Snow && self.snow_meets_liquid(cell, index, below) {
            return;
        }
        let target = if self.free_for_material(below, player) {
            self.flow[index].y = (self.flow[index].y - FALL_ACCELERATION).max(-MAX_CELL_SPEED);
            self.phase[index].y -= self.flow[index].y;
            let steps = (self.phase[index].y as i32).clamp(1, MAX_CELL_SPEED as i32);
            self.phase[index].y = (self.phase[index].y - steps as f32).max(0.0);
            let mut target = below;
            for _ in 1..steps {
                let next = GridPosition { x, y: target.y - 1 };
                if !self.free_for_material(next, player) {
                    break;
                }
                target = next;
            }
            Some(target)
        } else {
            self.flow[index] = Vec2::ZERO;
            self.phase[index] = Vec2::ZERO;
            let side = if self.roll(cell, 21) & 1 == 0 { 1 } else { -1 };
            (!kind.is_cohesive())
                .then(|| {
                    [side, -side].into_iter().find_map(|side| {
                        let diagonal = GridPosition {
                            x: x + side,
                            y: y - 1,
                        };
                        // Snow lying as the wind laid it holds a steeper
                        // bank than sand: it slides only where the slope
                        // drops away by more than a step, so drifts keep
                        // their sculpted edges. Snow that has been kicked
                        // up and has landed again, or a lone grain standing
                        // up out of the surface, settles like any powder,
                        // so footsteps leave no lumps.
                        let spike = self.free_for_material(GridPosition { x: x - 1, y }, player)
                            && self.free_for_material(GridPosition { x: x + 1, y }, player);
                        let steep = kind != TerrainKind::Snow
                            || self.carried[index]
                            || spike
                            || self.free_for_material(
                                GridPosition {
                                    x: x + side,
                                    y: y - 2,
                                },
                                player,
                            );
                        // Do not pass diagonally through a sealed corner.
                        (steep
                            && self.free_for_material(GridPosition { x: x + side, y }, player)
                            && self.free_for_material(diagonal, player))
                        .then_some(diagonal)
                    })
                })
                .flatten()
        };
        if let Some(target) = target {
            self.swap_cells(cell, target);
            for neighbor in Self::neighbors(cell) {
                if self.get(neighbor).is_some_and(|kind| {
                    kind.is_solid() && !kind.is_powder() && kind != TerrainKind::Bedrock
                }) {
                    self.support_dirty.push(neighbor);
                }
            }
            if kind.loose() != kind {
                let index = self.index(target).unwrap();
                self.cells[index] = Some(kind.loose());
                self.base_colors[index] = kind.loose().color(target.x, target.y);
                self.carried[index] = false;
                self.refresh_activity(index);
                self.refresh_reaction_neighbors(target);
            }
        }
    }

    /// Snow resting on liquid: on water the two freeze together into ice
    /// (snow falling on open water skins it over); on lava it flashes to
    /// steam; into anything else it melts. Whether it did.
    fn snow_meets_liquid(&mut self, cell: GridPosition, index: usize, below: GridPosition) -> bool {
        let Some(liquid) = self.get(below).filter(|kind| kind.is_liquid()) else {
            return false;
        };
        if !self.roll(cell, 37).is_multiple_of(4) {
            // It sits on the surface a moment first.
            self.keep_awake(cell);
            return true;
        }
        if liquid == TerrainKind::Water {
            for (at, slot) in [(cell, Some(index)), (below, self.index(below))] {
                self.put(at, Some(TerrainKind::Ice));
                if let Some(slot) = slot {
                    self.temperature[slot] = -4.0;
                    self.track_heat(slot);
                }
            }
            return true;
        }
        let into = if liquid == TerrainKind::Lava {
            TerrainKind::Steam
        } else {
            TerrainKind::Water
        };
        self.put(cell, Some(into));
        self.temperature[index] = if into == TerrainKind::Steam {
            130.0
        } else {
            8.0
        };
        self.track_heat(index);
        true
    }

    /// Settle where the grain is, or in the open cell it bounced back into.
    /// Never search through a wall for space.
    pub(super) fn deposit(&mut self, grain: &Grain, back: Vec2, player: Option<Vec2>) -> bool {
        // Liquid flows around characters (see `LiquidBody`), so a drop may
        // land where one stands: a drip from its feet, a splash at its side.
        let player = if grain.kind.is_liquid() { None } else { player };
        let here = self.world_to_cell(grain.position);
        let behind = self.world_to_cell(grain.position + back * self.pixel_size);
        let Some(cell) = [here, behind]
            .into_iter()
            .find(|&cell| self.free_for_material(cell, player))
        else {
            return false;
        };
        self.put(cell, Some(grain.kind));
        let index = self.index(cell).unwrap();
        self.base_colors[index] = grain.color;
        self.carried[index] = true;
        self.temperature[index] = grain.temperature;
        self.track_heat(index);
        self.life[index] = grain.fuel;
        if grain.kind.is_liquid() {
            self.flow[index] = (grain.velocity / (self.pixel_size * 60.0))
                .clamp(Vec2::splat(-2.0), Vec2::splat(2.0));
            self.flow[index].y = self.flow[index].y.min(0.0);
        }
        true
    }

    /// Shade of the pixel at `i` once it breaks loose: its own if it stays the
    /// same material, otherwise the loose material's (stone becomes rubble).
    pub(super) fn loose_color(&self, i: usize, cell: GridPosition) -> [u8; 4] {
        let color = match self.cells[i] {
            Some(kind) if kind.loose() != kind => kind.loose().color(cell.x, cell.y),
            _ => self.base_colors[i],
        };
        // Bloodied rock breaks into bloodied rubble.
        render::stained(color, self.stain[i])
    }

    /// An open cell directly above or beside `cell`, checked in that order.
    pub(super) fn open_beside(
        &self,
        cell: GridPosition,
        player: Option<Vec2>,
    ) -> Option<GridPosition> {
        [(0, 1), (-1, 0), (1, 0)]
            .into_iter()
            .map(|(dx, dy)| GridPosition {
                x: cell.x + dx,
                y: cell.y + dy,
            })
            .find(|&next| self.free_for_material(next, player))
    }

    /// For a grain come to rest inside the room the player's body holds
    /// (snow shed off it as it gets up, say): an open cell just past the
    /// nearer side of the body, a little up if need be, so the grain falls
    /// beside it at once instead of waiting there, jammed, and hanging in
    /// the air once the body has gone.
    pub(super) fn beside_player(
        &self,
        cell: GridPosition,
        player: Option<Vec2>,
    ) -> Option<GridPosition> {
        let body = player?;
        if !self.blocked_by_player(cell, player) {
            return None;
        }
        let at = self.cell_to_world(cell);
        let nearer = if at.x < body.x { -1.0 } else { 1.0 };
        let out = PLAYER_HALF_SIZE.x + self.pixel_size * 1.5;
        [nearer, -nearer].into_iter().find_map(|side| {
            let start = self.world_to_cell(Vec2::new(body.x + side * out, at.y));
            (0..=6)
                .map(|dy| GridPosition {
                    x: start.x,
                    y: start.y + dy,
                })
                .find(|&next| self.free_for_material(next, player) && !self.grain_at(next))
        })
    }

    /// The first open cell straight above `start` (or in the column beside
    /// it) that can be reached by climbing through loose material: powder,
    /// liquid or other debris. Never passes through rock.
    pub(super) fn open_above_loose(
        &self,
        start: GridPosition,
        player: Option<Vec2>,
    ) -> Option<GridPosition> {
        [0, -1, 1].into_iter().find_map(|dx| {
            (1..=CLIMB_REACH)
                .map(|dy| GridPosition {
                    x: start.x + dx,
                    y: start.y + dy,
                })
                .take_while(|&cell| {
                    self.index(cell).is_some()
                        && (self.free_for_material(cell, player)
                            || self.grain_at(cell)
                            || self
                                .get(cell)
                                .is_some_and(|kind| kind.is_powder() || kind.is_liquid()))
                })
                .find(|&cell| self.free_for_material(cell, player))
        })
    }

    /// Nearest open cell reachable from `start` through loose material (powder,
    /// liquid, or other debris), preferring upward. Never passes through rock.
    pub(super) fn open_through_loose(
        &self,
        start: GridPosition,
        player: Option<Vec2>,
    ) -> Option<GridPosition> {
        let mut queue = std::collections::VecDeque::from([start]);
        let mut seen = std::collections::HashSet::from([start]);
        while let Some(cell) = queue.pop_front() {
            if seen.len() > 512 {
                break;
            }
            for (dx, dy) in [(0, 1), (-1, 0), (1, 0), (0, -1)] {
                let next = GridPosition {
                    x: cell.x + dx,
                    y: cell.y + dy,
                };
                if self.index(next).is_none() || !seen.insert(next) {
                    continue;
                }
                if self.free_for_material(next, player) {
                    return Some(next);
                }
                if self.grain_at(next)
                    || self
                        .get(next)
                        .is_some_and(|kind| kind.is_powder() || kind.is_liquid())
                {
                    queue.push_back(next);
                }
            }
        }
        None
    }

    pub fn step_grains(&mut self, dt: f32, gravity: f32, player: Option<Vec2>) {
        let mut grains = std::mem::take(&mut self.grains);
        let had_grains = !grains.is_empty();
        let live = self.live_cells();
        grains.retain_mut(|grain| {
            // Beyond the live window a grain hangs where it is, still holding
            // its cell, until the player comes back.
            if !self.in_live(live, self.world_to_cell(grain.position)) {
                let chunk = self.chunk_of(grain.position);
                self.parked.entry(chunk).or_default().push(*grain);
                self.parked_count += 1;
                return false;
            }
            // A grain wedged where it cannot settle rests a while before
            // looking for room again, instead of searching every tick.
            if grain.rest > 0 {
                grain.rest -= 1;
                return true;
            }
            // Reserve moving material too, so powders and other grains cannot
            // overwrite its cell. Release only this grain while sweeping it.
            self.reserve_grain(grain.position, None);
            // If material ever ends up in a grain's cell, step out into an
            // adjacent open cell rather than staying buried in it.
            let here = self.world_to_cell(grain.position);
            if self.get(here).is_some_and(|kind| !kind.is_gas())
                && let Some(exit) = self.open_beside(here, player)
            {
                grain.position = self.cell_to_world(exit);
            }
            let surrounding_heat = self.temperature_at(grain.position);
            grain.temperature += (surrounding_heat - grain.temperature) * dt * 0.4;
            grain.temperature += (20.0 - grain.temperature) * dt * 0.8;
            if self.react_grain(grain, player) {
                return false;
            }
            grain.velocity.y = (grain.velocity.y - gravity * dt).max(-850.0);
            if grain.kind.is_liquid() {
                // Drips and spray lose speed to air in both axes. The speed
                // term creates a terminal fall without an abrupt velocity cap;
                // oil's thicker drops are slowed a little more than water.
                let (linear, quadratic) = if grain.kind.is_aqueous() {
                    (2.4, 0.008)
                } else {
                    (3.0, 0.011)
                };
                grain.velocity /= 1.0 + dt * (linear + quadratic * grain.velocity.length());
            } else {
                grain.velocity.x *= 1.0 / (1.0 + 0.35 * dt);
            }
            let end = grain.position + grain.velocity * dt;
            // Grains sharing a cell (after a ghosted flight) must be free to part.
            let solid_with_others =
                grain.ghost == 0 && !self.grain_at(self.world_to_cell(grain.position));
            grain.ghost = grain.ghost.saturating_sub(1);
            let Some(hit) = self.sweep_material(
                grain.position,
                end,
                self.pixel_size * 0.35,
                true,
                solid_with_others,
            ) else {
                grain.position = end;
                self.reserve_grain(grain.position, Some(grain.kind));
                return true;
            };
            // Back off the contact slightly, but never across a cell edge into
            // the material that was hit.
            let nudged = hit.position + hit.normal * 0.02;
            grain.position = if self.material_at(nudged).is_some_and(|kind| !kind.is_gas()) {
                hit.position
            } else {
                nudged
            };
            let speed = grain.velocity.length();
            let contact = hit.position - hit.normal * self.pixel_size * 0.85;
            if grain.kind.is_liquid() {
                let into_pool = self
                    .material_at(contact)
                    .is_some_and(TerrainKind::is_liquid);
                if self.deposit(grain, hit.normal, player) {
                    self.disturb_surface(grain.position, speed / 260.0);
                    if speed > 60.0 {
                        let noise = if into_pool {
                            soundscape::Noise::Plip
                        } else {
                            soundscape::Noise::Pat
                        };
                        self.hear(noise, self.world_to_cell(grain.position), speed / 300.0);
                    }
                    return false;
                }
            } else if self
                .material_at(contact)
                .is_some_and(TerrainKind::is_liquid)
            {
                // Debris plops onto the surface and sinks through the pool.
                self.disturb_surface(contact, speed / 220.0);
                if self.deposit(grain, hit.normal, player) {
                    return false;
                }
            }
            let bounce = grain.kind.bounce();
            let incoming = grain.velocity.dot(hit.normal);
            if incoming < -90.0 && grain.kind.is_solid() {
                self.hear(
                    soundscape::Noise::Debris,
                    self.world_to_cell(grain.position),
                    -incoming / 300.0,
                );
            }
            if incoming < 0.0 {
                let normal = hit.normal * incoming;
                let tangent = grain.velocity - normal;
                grain.velocity = tangent * bounce.friction - normal * bounce.restitution;
            }
            if grain.velocity.length() < bounce.settle_speed {
                if self.deposit(grain, hit.normal, player) {
                    return false;
                }
                // Jammed in a cell shared with other resting debris (grains
                // sharing a cell block each other from settling in it): shuffle
                // into an adjacent open cell, or, if loose material has closed
                // over the pocket, climb up through that pile to open air,
                // straight up first (cheap however deep the pile is), then by a
                // short search around. Sealed in, it waits a while.
                let here = self.world_to_cell(grain.position);
                match self
                    .open_beside(here, player)
                    .or_else(|| self.beside_player(here, player))
                    .or_else(|| self.open_above_loose(here, player))
                    .or_else(|| self.open_through_loose(here, player))
                {
                    Some(open) => grain.position = self.cell_to_world(open),
                    None => grain.rest = JAMMED_REST,
                }
            }
            self.reserve_grain(grain.position, Some(grain.kind));
            true
        });
        // Splashes raised while grains landed were queued on the (empty)
        // live list; keep them.
        let splashed = std::mem::replace(&mut self.grains, grains);
        self.grains.extend(splashed);
        self.dirty |= had_grains;
    }
}
