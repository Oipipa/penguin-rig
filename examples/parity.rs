//! Fixed-tick cell hashes for comparing this extraction with Penguin.

use penguin_sim::{GridPosition, LevelGrid, TerrainKind, Tuning};

fn cell(x: i32, y: i32) -> GridPosition {
    GridPosition { x, y }
}
fn fill(grid: &mut LevelGrid, x0: i32, y0: i32, width: i32, height: i32, kind: TerrainKind) {
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            grid.put(cell(x, y), Some(kind));
        }
    }
}
fn hash(grid: &LevelGrid) -> u64 {
    let mut h = 0xcbf29ce484222325_u64;
    for y in 0..grid.height {
        for x in 0..grid.width {
            let at = cell(x, y);
            let i = grid.index(at).unwrap();
            for byte in [grid.cells[i].map_or(255, |k| k as u8)]
                .into_iter()
                .chain(grid.temperature[i].to_bits().to_le_bytes())
                .chain([grid.airborne[i].map_or(255, |k| k as u8)])
            {
                h = (h ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
    }
    h
}
fn run(name: &str, setup: impl FnOnce(&mut LevelGrid), ticks: usize, expected: u64) {
    let tuning = Tuning {
        tile_size: 2.0,
        arena_columns: 64,
        arena_rows: 64,
        pixels_per_tile: 1,
        ..Tuning::default()
    };
    let mut grid = LevelGrid::new(&tuning);
    setup(&mut grid);
    for _ in 0..ticks {
        grid.step_materials(1.0 / 60.0, 1400.0, None);
    }
    let actual = hash(&grid);
    assert_eq!(actual, expected, "{name} diverged from Penguin");
    println!(
        "{name} {ticks} {:016x} {} {}",
        actual,
        grid.grains.len(),
        grid.motes.len()
    );
}
fn main() {
    run(
        "powder",
        |grid| {
            fill(grid, 0, 0, 64, 3, TerrainKind::Bedrock);
            fill(grid, 9, 19, 28, 3, TerrainKind::Stone);
            fill(grid, 18, 48, 17, 9, TerrainKind::Sand);
            fill(grid, 39, 45, 12, 6, TerrainKind::Snow);
        },
        90,
        0xda9d70b634939509,
    );
    run(
        "liquid",
        |grid| {
            fill(grid, 0, 0, 64, 3, TerrainKind::Bedrock);
            fill(grid, 4, 3, 2, 48, TerrainKind::Stone);
            fill(grid, 50, 3, 2, 48, TerrainKind::Stone);
            fill(grid, 6, 3, 23, 25, TerrainKind::Water);
            fill(grid, 29, 3, 19, 17, TerrainKind::Oil);
        },
        120,
        0x55b9e16212e4b4ec,
    );
    run(
        "heat",
        |grid| {
            fill(grid, 0, 0, 64, 3, TerrainKind::Bedrock);
            fill(grid, 9, 8, 9, 9, TerrainKind::Ice);
            fill(grid, 20, 8, 9, 9, TerrainKind::Wood);
            fill(grid, 31, 8, 7, 7, TerrainKind::Lava);
            fill(grid, 40, 8, 8, 8, TerrainKind::Water);
            fill(grid, 21, 17, 5, 2, TerrainKind::Fire);
        },
        90,
        0xd329c815ee7dd414,
    );
    run(
        "reaction",
        |grid| {
            fill(grid, 0, 0, 64, 3, TerrainKind::Bedrock);
            fill(grid, 7, 3, 20, 20, TerrainKind::Lava);
            fill(grid, 7, 23, 20, 12, TerrainKind::Water);
            fill(grid, 35, 3, 13, 15, TerrainKind::Metal);
            fill(grid, 35, 18, 13, 12, TerrainKind::Acid);
        },
        75,
        0xeff4ed017dc0befa,
    );
}
