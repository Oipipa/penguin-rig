//! The pressure pass: connected liquid levels out through the cells
//! between, so communicating vessels fill to the same height.

use super::*;

impl LevelGrid {
    /// Resting liquid transmits pressure; falling liquid does not.
    pub(super) fn resting_liquid(&self, i: usize) -> bool {
        if self.flow[i].y.abs() >= 0.05 {
            return false;
        }
        let cell = self.cell_of(i);
        let below = GridPosition {
            x: cell.x,
            y: cell.y - 1,
        };
        // Most candidates are inside a pool, with liquid or rock directly
        // below. They need no body, grain or world-space occupancy query.
        self.step_index(i, cell, 0, -1)
            .is_some_and(|j| self.cells[j].is_some_and(|kind| !kind.is_gas()))
            || !self.open_for_liquid(below)
    }

    /// Hydrostatic transfer through each connected, resting body of liquid.
    /// The local rules already carry liquid downhill and level the surface; this
    /// only does what they cannot. Liquid trapped under a ceiling rises up the far
    /// side of the bend (communicating vessels) at a rate set by the height
    /// difference, and a slow trickle irons out wide, shallow slopes.
    pub(super) fn equalize_pressure(&mut self, player: Option<Vec2>) {
        // Only pools whose shape changed recently can have a head to relieve;
        // a settled pool was already level when it went to sleep.
        let mut scratch = std::mem::take(&mut self.active.pressure_work);
        let activity::PressureWork {
            liquids,
            region,
            sources,
            targets,
        } = &mut scratch;
        liquids.clear();
        for (x0, y0) in self.sleep.pressure_chunks(&self.chunks, self.tick) {
            if let Some(slot) = self
                .chunks
                .slot(x0 / sleep::SIM_CHUNK, y0 / sleep::SIM_CHUNK)
            {
                // Preserve the old bottom-up, left-to-right seed order while
                // visiting occupied cells rather than all 1,024 positions.
                self.active.liquids.extend_slot_ordered(slot, liquids);
            }
        }
        self.visit_stamp = self.visit_stamp.wrapping_add(1);
        if self.visit_stamp == 0 {
            self.visited.fill(0);
            self.visit_stamp = 1;
        }
        let stamp = self.visit_stamp;
        let live = self.live_cells();
        #[cfg(test)]
        let mut region_time = [std::time::Duration::ZERO; 3];
        #[cfg(test)]
        let mut region_cells = [0_u64; 3];
        #[cfg(test)]
        let mut transfers = 0_u64;
        for &seed in liquids.iter() {
            if self.visited[seed] == stamp || !self.resting_liquid(seed) {
                continue;
            }
            let Some(kind) = self.cells[seed].filter(|kind| kind.is_liquid()) else {
                continue;
            };
            #[cfg(test)]
            let region_started = std::time::Instant::now();
            // Liquid and open cells are disjoint, so one stamp marks both the
            // region's cells and the open cells already listed as targets.
            self.visited[seed] = stamp;
            region.clear();
            region.push(seed);
            sources.clear();
            targets.clear();
            let mut lowest_ceiling = i32::MAX;
            let mut cursor = 0;
            while cursor < region.len() {
                let i = region[cursor];
                cursor += 1;
                let cell = self.cell_of(i);
                let above = GridPosition {
                    x: cell.x,
                    y: cell.y + 1,
                };
                match self.step_index(i, cell, 0, 1).and_then(|j| self.cells[j]) {
                    Some(roof) if roof.is_solid() => lowest_ceiling = lowest_ceiling.min(cell.y),
                    Some(roof) if roof.is_liquid() => {}
                    _ if !self.body_at(cell) && !self.body_at(above) => sources.push(cell),
                    _ => {}
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let next = GridPosition {
                        x: cell.x + dx,
                        y: cell.y + dy,
                    };
                    let Some(j) = self.step_index(i, cell, dx, dy) else {
                        continue;
                    };
                    // Beyond the live window the pool is frozen: pressure
                    // stops at its edge, so a lake costs what is in view.
                    if self.visited[j] == stamp
                        || !live.contains(next)
                        || self.airborne[j].is_some()
                        || self.body_at(next)
                    {
                        continue;
                    }
                    if self.cells[j].is_some_and(|other| other.pools_with(kind)) {
                        if self.resting_liquid(j) {
                            self.visited[j] = stamp;
                            region.push(j);
                        }
                    } else if self.cells[j].is_none_or(TerrainKind::is_gas)
                        && !self.open_for_liquid(GridPosition {
                            x: next.x,
                            y: next.y - 1,
                        })
                    {
                        self.visited[j] = stamp;
                        targets.push(next);
                    }
                }
            }
            #[cfg(test)]
            {
                let group = match kind {
                    TerrainKind::Lava => 0,
                    TerrainKind::Water => 1,
                    _ => 2,
                };
                region_time[group] += region_started.elapsed();
                region_cells[group] += region.len() as u64;
            }
            if sources.is_empty() || targets.is_empty() {
                continue;
            }
            sources.sort_unstable_by_key(|&p| (-p.y, self.roll(p, 13)));
            targets.sort_unstable_by_key(|&p| (p.y, self.roll(p, 14)));
            let head = sources[0].y - targets[0].y;
            if head < 1 {
                continue;
            }
            // A route that dips under a ceiling must lift liquid (a U-bend), which
            // the local rules never do, so pressure carries it across the whole
            // body. In open pools, pressure only hurries the local slope along:
            // a few surface pixels per tick move to lower open spots nearby.
            let water = kind.is_aqueous();
            let confined = lowest_ceiling < targets[0].y;
            let (budget, reach) = if confined {
                ((head / 2).clamp(1, if water { 6 } else { 2 }), i32::MAX)
            } else if head <= 2 {
                // Ironing out the last shallow terraces: one pixel at a time,
                // anywhere in the pool.
                (1, i32::MAX)
            } else if water {
                ((head / 3).clamp(1, 14), OPEN_PRESSURE_REACH * 2)
            } else {
                ((head / 5).clamp(1, 3), OPEN_PRESSURE_REACH)
            };
            let mut moved = 0;
            for &from in sources.iter() {
                if moved >= budget {
                    break;
                }
                let Some(slot) = targets.iter().position(|to| {
                    from.y > to.y
                        && (to.x - from.x).abs() <= reach
                        && self.fluid_destination(from, *to, kind, player)
                }) else {
                    continue;
                };
                let to = targets.swap_remove(slot);
                targets.sort_unstable_by_key(|&p| (p.y, self.roll(p, 14)));
                self.swap_cells(from, to);
                let i = self.index(to).unwrap();
                self.flow[i] = Vec2::ZERO;
                moved += 1;
                #[cfg(test)]
                {
                    transfers += 1;
                }
            }
        }
        #[cfg(test)]
        for (group, label) in ["lava", "water", "other"].into_iter().enumerate() {
            let name = match label {
                "lava" => "    pressure lava regions",
                "water" => "    pressure water regions",
                _ => "    pressure other regions",
            };
            crate::dev::profile::add(name, region_time[group]);
            let name = match label {
                "lava" => "    pressure lava cells (count)",
                "water" => "    pressure water cells (count)",
                _ => "    pressure other cells (count)",
            };
            crate::dev::profile::add(name, std::time::Duration::from_micros(region_cells[group]));
        }
        #[cfg(test)]
        crate::dev::profile::add(
            "    pressure transfers (count)",
            std::time::Duration::from_micros(transfers),
        );
        self.active.pressure_work = scratch;
    }
}
