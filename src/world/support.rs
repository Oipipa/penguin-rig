//! Small detached solid chips next to an edit lose their support and fall
//! as debris; connected rock stays put.

use super::*;
use std::collections::HashSet;

impl LevelGrid {
    /// Revisit only material bordering an edit. Small severed chips fall as real
    /// material; large connected structures stay cohesive (no rigid-body solver).
    pub(super) fn settle_detached(&mut self) {
        let seeds = std::mem::take(&mut self.support_dirty);
        let mut checked = HashSet::new();
        for seed in seeds {
            if checked.contains(&seed)
                || !self.get(seed).is_some_and(|kind| {
                    kind.is_solid() && !kind.is_powder() && kind != TerrainKind::Bedrock
                })
            {
                continue;
            }
            let mut component = vec![seed];
            let mut seen = HashSet::from([seed]);
            let mut anchored = false;
            let mut cursor = 0;
            while cursor < component.len() && component.len() <= 64 {
                let cell = component[cursor];
                cursor += 1;
                if cell.y <= 0 || cell.x <= 0 || cell.x >= self.width - 1 {
                    anchored = true;
                    break;
                }
                for next in Self::neighbors(cell) {
                    let Some(kind) = self.get(next).filter(|kind| kind.is_solid()) else {
                        continue;
                    };
                    if kind == TerrainKind::Bedrock || kind.is_powder() {
                        anchored = true;
                        break;
                    }
                    if seen.insert(next) {
                        component.push(next);
                    }
                }
                if anchored {
                    break;
                }
            }
            checked.extend(component.iter().copied());
            if anchored || component.len() > 64 {
                continue;
            }
            for cell in component {
                let i = self.index(cell).unwrap();
                let kind = self.cells[i].unwrap();
                let grain = Grain {
                    position: self.cell_to_world(cell),
                    velocity: Vec2::ZERO,
                    kind: kind.loose(),
                    temperature: self.temperature[i],
                    fuel: self.life[i],
                    color: self.loose_color(i, cell),
                    ghost: 0,
                    rest: 0,
                };
                self.put(cell, None);
                self.reserve_grain(grain.position, Some(grain.kind));
                self.grains.push(grain);
            }
        }
    }
}
