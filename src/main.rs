//! Interactive editor for the standalone material simulation.

use eframe::egui::{
    self, Color32, ColorImage, PointerButton, Sense, TextureHandle, TextureOptions, Vec2,
};
use penguin_sim::{GridPosition, Scene, Simulation, TerrainKind};
use std::time::Instant;

const PRESETS: [(&str, &str); 4] = [
    ("Powders", include_str!("../scenes/powders.toml")),
    ("Liquids", include_str!("../scenes/liquids.toml")),
    ("Fire", include_str!("../scenes/fire.toml")),
    ("Reactions", include_str!("../scenes/reactions.toml")),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Paint,
    Erase,
    Heat,
}

struct Editor {
    sim: Simulation,
    texture: Option<TextureHandle>,
    selected: TerrainKind,
    tool: Tool,
    brush: i32,
    heat: f32,
    zoom: f32,
    paused: bool,
    path: String,
    status: String,
    last_frame: Instant,
    accumulator: f64,
}

impl Editor {
    fn new(scene: Scene, path: String) -> Self {
        let zoom = scene.zoom;
        Self {
            sim: Simulation::new(scene).expect("validated startup scene"),
            texture: None,
            selected: TerrainKind::Sand,
            tool: Tool::Paint,
            brush: 3,
            heat: 80.0,
            zoom,
            paused: false,
            path,
            status: String::new(),
            last_frame: Instant::now(),
            accumulator: 0.0,
        }
    }

    fn load_scene(&mut self, scene: Scene) {
        let zoom = scene.zoom;
        match Simulation::new(scene) {
            Ok(sim) => {
                self.sim = sim;
                self.zoom = zoom;
                self.texture = None;
                self.accumulator = 0.0;
                self.status = "Scene loaded".into();
            }
            Err(error) => self.status = error,
        }
    }

    fn tick_clock(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame).as_secs_f64().min(0.1);
        self.last_frame = now;
        if self.paused {
            return;
        }
        self.accumulator += elapsed * self.sim.speed as f64 * self.sim.tick_hz as f64;
        let steps = (self.accumulator as u32).min(8);
        self.sim.step_n(steps);
        self.accumulator -= steps as f64;
        // A slow machine should not spend its next frame catching up forever.
        self.accumulator = self.accumulator.min(8.0);
    }

    fn draw_grid(&mut self, ctx: &egui::Context) {
        let grid = self.sim.grid();
        let width = grid.width as usize;
        let height = grid.height as usize;
        let mut pixels = Vec::with_capacity(width * height);
        for row in 0..height {
            let y = grid.height - 1 - row as i32;
            for x in 0..grid.width {
                let cell = GridPosition { x, y };
                let index = grid.index(cell);
                let kind = grid
                    .get(cell)
                    .or_else(|| index.and_then(|i| grid.airborne[i]));
                let color = match (kind, index) {
                    (Some(kind), Some(i)) if grid.cells[i] == Some(kind) => grid.base_colors[i],
                    (Some(kind), _) => kind.color(x, y),
                    _ => [14, 20, 29, 255],
                };
                let alpha = color[3] as u32;
                let blend = |c: usize, bg: u8| {
                    ((color[c] as u32 * alpha + bg as u32 * (255 - alpha)) / 255) as u8
                };
                pixels.push(Color32::from_rgb(blend(0, 14), blend(1, 20), blend(2, 29)));
            }
        }
        let image = ColorImage::new([width, height], pixels);
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::NEAREST),
            None => {
                self.texture =
                    Some(ctx.load_texture("material-grid", image, TextureOptions::NEAREST))
            }
        }
    }

    fn apply_brush(&mut self, center: GridPosition, override_tool: Option<Tool>) {
        let radius = self.brush - 1;
        let width = self.sim.grid().width;
        let height = self.sim.grid().height;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                let cell = GridPosition {
                    x: center.x + dx,
                    y: center.y + dy,
                };
                if cell.x < 0 || cell.y < 0 || cell.x >= width || cell.y >= height {
                    continue;
                }
                match override_tool.unwrap_or(self.tool) {
                    Tool::Paint => self.sim.paint(cell, Some(self.selected)),
                    Tool::Erase => self.sim.paint(cell, None),
                    Tool::Heat => self.sim.heat(cell, self.heat),
                }
            }
        }
    }
}

impl eframe::App for Editor {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick_clock();
        self.draw_grid(ctx);
        egui::SidePanel::left("controls").default_width(220.0).show(ctx, |ui| {
            ui.heading("Penguin Sim");
            ui.label(format!("Tick {}", self.sim.ticks()));
            ui.horizontal(|ui| {
                if ui.button(if self.paused { "Play" } else { "Pause" }).clicked() {
                    self.paused = !self.paused;
                }
                if ui.button("Step").clicked() { self.sim.step(); }
                if ui.button("Reset").clicked() { self.sim.reset(); }
            });
            ui.separator();
            ui.label("Simulation");
            ui.add(egui::Slider::new(&mut self.sim.gravity, 0.0..=3000.0).text("Gravity"));
            ui.add(egui::Slider::new(&mut self.sim.tick_hz, 1.0..=120.0).text("Tick Hz"));
            ui.add(egui::Slider::new(&mut self.sim.speed, 0.05..=8.0).text("Speed"));
            ui.checkbox(&mut self.sim.weather, "Snow and wind");
            ui.separator();
            ui.label("Brush");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tool, Tool::Paint, "Paint");
                ui.selectable_value(&mut self.tool, Tool::Erase, "Erase");
                ui.selectable_value(&mut self.tool, Tool::Heat, "Heat");
            });
            ui.add(egui::Slider::new(&mut self.brush, 1..=12).text("Radius"));
            if self.tool == Tool::Heat {
                ui.add(egui::Slider::new(&mut self.heat, -200.0..=300.0).text("Degrees"));
            }
            ui.add(egui::Slider::new(&mut self.zoom, 1.0..=8.0).text("Zoom"));
            egui::ScrollArea::vertical().max_height(245.0).show(ui, |ui| {
                for kind in TerrainKind::ALL {
                    let color = kind.color(17, 17);
                    ui.horizontal(|ui| {
                        ui.colored_label(Color32::from_rgb(color[0], color[1], color[2]), "■");
                        ui.selectable_value(&mut self.selected, kind, format!("{kind:?}"));
                    });
                }
            });
            ui.separator();
            ui.label("Scenes");
            for (name, text) in PRESETS {
                if ui.button(name).clicked() {
                    match Scene::from_toml(text) {
                        Ok(scene) => self.load_scene(scene),
                        Err(error) => self.status = error,
                    }
                }
            }
            ui.text_edit_singleline(&mut self.path);
            ui.horizontal(|ui| {
                if ui.button("Load TOML").clicked() {
                    match Scene::load(&self.path) {
                        Ok(scene) => self.load_scene(scene),
                        Err(error) => self.status = error,
                    }
                }
                if ui.button("Save TOML").clicked() {
                    let mut scene = self.sim.snapshot_scene();
                    scene.zoom = self.zoom;
                    match scene.save(&self.path) {
                        Ok(()) => self.status = format!("Saved {}", self.path),
                        Err(error) => self.status = error,
                    }
                }
            });
            if !self.status.is_empty() { ui.label(&self.status); }
            ui.small("Left drag paints; right drag erases. Saved scenes contain settled cells and temperatures.");
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                if let Some(texture) = &self.texture {
                    let size = Vec2::new(
                        self.sim.grid().width as f32 * self.zoom,
                        self.sim.grid().height as f32 * self.zoom,
                    );
                    let response = ui
                        .add(egui::Image::new((texture.id(), size)).sense(Sense::click_and_drag()));
                    let left = ui.input(|i| i.pointer.button_down(PointerButton::Primary));
                    let right = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
                    if response.hovered()
                        && (left || right)
                        && let Some(pos) = response.interact_pointer_pos()
                    {
                        let x = ((pos.x - response.rect.left()) / self.zoom).floor() as i32;
                        let row = ((pos.y - response.rect.top()) / self.zoom).floor() as i32;
                        let cell = GridPosition {
                            x,
                            y: self.sim.grid().height - 1 - row,
                        };
                        if right {
                            self.apply_brush(cell, Some(Tool::Erase));
                        } else {
                            self.apply_brush(cell, None);
                        }
                    }
                }
            });
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn main() -> eframe::Result {
    let (path, scene) = if let Some(path) = std::env::args().nth(1) {
        let scene = match Scene::load(&path) {
            Ok(scene) => scene,
            Err(error) => {
                eprintln!("{path}: {error}");
                std::process::exit(2);
            }
        };
        (path, scene)
    } else {
        (
            "my_scene.toml".into(),
            Scene::from_toml(PRESETS[0].1).expect("bundled preset is valid"),
        )
    };
    eframe::run_native(
        "Penguin Sim",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1120.0, 760.0]),
            ..Default::default()
        },
        Box::new(move |_cc| Ok(Box::new(Editor::new(scene, path)))),
    )
}
