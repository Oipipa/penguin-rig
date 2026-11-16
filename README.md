# Penguin Sim

A standalone terrain and material sandbox extracted from Penguin. It contains the 28 materials and their cell physics: powders, liquids, gases, heat, fire, phase changes, reactions, falling debris, and weather. There are no levels, story scenes, creatures, combat, saves, or game runtime. The source is an independent snapshot of Penguin commit `364199b`.

## Run

Requires Rust 1.98.1 or newer and a Linux desktop with X11 or Wayland. The repo limits Cargo to two build jobs and uses a small dev profile to suit a machine with limited available RAM.

```sh
cargo run
cargo run -- scenes/reactions.toml
```

The first command opens the Powders preset. The second opens a TOML scene. The first build takes longer than later ones.

## Editor

- **Paint / Erase / Heat:** choose a tool, material, brush radius, and heat amount in the left panel. Left drag applies the tool; right drag erases.
- **Play / Pause / Step / Reset:** control fixed simulation ticks. Reset reloads the scene that was last loaded or selected.
- **Gravity / Tick Hz / Speed:** changes take effect immediately. Tick Hz changes the physics time step; Speed changes playback rate. The Penguin defaults are 1400 gravity and 60 Hz.
- **Zoom:** scales the grid. Scroll to inspect larger grids.
- **Scenes:** choose a preset, or enter a TOML path and press Load or Save. Save captures settled cells and their temperatures; in-flight grains and motes are omitted.

## Scene format

```toml
width = 256
height = 160
gravity = 1400.0
tick_hz = 60.0
speed = 1.0
zoom = 3.0
weather = false

[[regions]]
x = 0
y = 0
width = 256
height = 8
material = "Bedrock"

[[regions]]
x = 60
y = 100
width = 30
height = 10
material = "Water"
temperature = 50.0
```

Coordinates start at the bottom left. Regions later in the file overwrite earlier ones. Material names ignore case, spaces, hyphens, and underscores. Grid width and height must each be 32–512 cells. The four bundled scenes live in `scenes/`.

## Library

`Scene::from_toml` or `Scene::load` validates settings and placements. `Simulation::new` builds the grid. Use `get`, `paint`, `heat`, `set_temperature`, `step`, `step_n`, `reset`, and `snapshot_scene` to drive it without the window. `LevelGrid` and `TerrainKind` are also public for lower-level inspection. The default step uses the same material order, 60 Hz, and gravity as Penguin.

## Checks

```sh
cargo fmt --check
cargo check
cargo test --lib
cargo run --example parity
```

The `parity` example prints fixed-tick hashes for powder, liquid, heat, and reaction scenes. It can be compiled against Penguin's engine in a separate temporary harness to compare results without modifying Penguin.
