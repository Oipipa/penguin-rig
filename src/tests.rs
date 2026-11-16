//! Tests for the standalone API and representative material behavior.

use crate::{GridPosition, Region, Scene, Simulation, TerrainKind, material_named};

fn cell(x: i32, y: i32) -> GridPosition {
    GridPosition { x, y }
}
fn region(x: i32, y: i32, width: i32, height: i32, material: &str) -> Region {
    Region {
        x,
        y,
        width,
        height,
        material: material.into(),
        temperature: None,
    }
}
fn scene(regions: Vec<Region>) -> Scene {
    Scene {
        width: 64,
        height: 64,
        regions,
        ..Scene::default()
    }
}

#[test]
fn every_material_has_a_scene_name() {
    for kind in TerrainKind::ALL {
        assert_eq!(material_named(&format!("{kind:?}")), Some(kind));
    }
    assert_eq!(
        material_named("volcanic_rock"),
        Some(TerrainKind::VolcanicRock)
    );
}

#[test]
fn scene_validation_rejects_invalid_input() {
    let bad = Scene {
        regions: vec![region(250, 20, 10, 1, "Water")],
        ..Scene::default()
    };
    assert!(bad.validate().unwrap_err().contains("outside"));
    let bad = Scene {
        regions: vec![region(0, 0, 1, 1, "Unobtainium")],
        ..Scene::default()
    };
    assert!(bad.validate().unwrap_err().contains("unknown material"));
    assert!(Scene::from_toml("width = 64\nheight = 64\ntick_hrz = 60").is_err());
    for (_, text) in PRESET_TEXTS {
        Scene::from_toml(text).unwrap();
    }
}

const PRESET_TEXTS: [(&str, &str); 4] = [
    ("powders", include_str!("../scenes/powders.toml")),
    ("liquids", include_str!("../scenes/liquids.toml")),
    ("fire", include_str!("../scenes/fire.toml")),
    ("reactions", include_str!("../scenes/reactions.toml")),
];

#[test]
fn powder_falls_and_liquid_stays_inside_walls() {
    let mut sim = Simulation::new(scene(vec![
        region(0, 0, 64, 2, "Bedrock"),
        region(10, 2, 2, 30, "Stone"),
        region(40, 2, 2, 30, "Stone"),
        region(20, 50, 1, 1, "Sand"),
        region(22, 2, 8, 8, "Water"),
    ]))
    .unwrap();
    sim.step_n(180);
    assert_eq!(
        sim.grid()
            .cells
            .iter()
            .filter(|&&k| k == Some(TerrainKind::Sand))
            .count(),
        1
    );
    assert!((2..=35).any(|y| sim.get(cell(20, y)) == Some(TerrainKind::Sand)));
    assert_eq!(
        sim.grid()
            .cells
            .iter()
            .filter(|&&k| k == Some(TerrainKind::Water))
            .count(),
        64
    );
    assert!((42..64).all(|x| (0..64).all(|y| sim.get(cell(x, y)) != Some(TerrainKind::Water))));
}

#[test]
fn heat_changes_phase_and_lava_reacts_with_water() {
    let mut sim = Simulation::new(scene(vec![
        region(0, 0, 64, 2, "Bedrock"),
        region(15, 12, 1, 1, "Ice"),
        region(30, 12, 1, 1, "Water"),
        region(45, 12, 1, 1, "Lava"),
        region(45, 11, 1, 1, "Water"),
    ]))
    .unwrap();
    sim.heat(cell(15, 12), 60.0);
    sim.heat(cell(30, 12), 200.0);
    sim.step();
    assert_eq!(sim.get(cell(15, 12)), Some(TerrainKind::Water));
    assert_eq!(sim.get(cell(30, 12)), Some(TerrainKind::Steam));
    sim.step_n(80);
    assert_ne!(sim.get(cell(45, 12)), Some(TerrainKind::Lava));
}

#[test]
fn snapshot_roundtrips_settled_cells_and_settings() {
    let mut sim = Simulation::new(scene(vec![region(5, 5, 3, 2, "Metal")])).unwrap();
    sim.gravity = 900.0;
    sim.speed = 2.0;
    sim.set_temperature(cell(5, 5), 300.0);
    let encoded = toml::to_string(&sim.snapshot_scene()).unwrap();
    let loaded = Scene::from_toml(&encoded).unwrap();
    let copy = Simulation::new(loaded).unwrap();
    assert_eq!(copy.get(cell(5, 5)), Some(TerrainKind::Metal));
    assert_eq!(copy.gravity, 900.0);
    assert_eq!(copy.speed, 2.0);
    assert_eq!(
        copy.grid()
            .temperature_at(copy.grid().cell_to_world(cell(5, 5))),
        300.0
    );
}

#[test]
fn weather_can_be_enabled_live() {
    let mut sim = Simulation::new(scene(vec![region(0, 0, 64, 2, "Bedrock")])).unwrap();
    sim.weather = true;
    sim.step_n(30);
    assert!(!sim.grid().motes.is_empty());
}
