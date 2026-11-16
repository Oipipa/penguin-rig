//! Sparks and motes over the terrain: spells' sparks (`Glow`), embers,
//! dust, bubbles, drips, fireflies (`ambient`), the First Stop's weather
//! (`weather`) and what the world's events throw up (`effects`).

use super::*;

mod ambient;
mod effects;
mod glow;
mod weather;

/// Every mote alive at once. Spells are made of sparks, so there is room
/// for a lot of them; the cosmetic reaction light has a budget of its own
/// ([`REACTION_MOTES`]) so an acid spill never crowds them out.
const MAX_MOTES: usize = 16_384;
/// Motes the chemistry may spend on its own light, froth and bubbles.
const REACTION_MOTES: usize = 4096;

/// What a spark is made of: the colors it burns through as it cools, how
/// hard it falls, how much the air slows it, how it bounces, and whether it
/// is hot enough to light what it touches.
///
/// Sparks are what Noita's spells are made of: not a picture of a bolt but
/// thousands of glowing pixels that fly, fall, skip off rock, pile up on
/// the floor for a moment and burn out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glow {
    /// Penga's spark bolt: white, ice blue, then a deep blue.
    BlueSpark,
    /// The spark bolt's glitter: white, then pink, then a deep violet.
    Arcane,
    /// Flame thrown as streaks: white-yellow, orange, a dull red.
    Fire,
    /// Rock melted by the drill: white-hot, orange, dark red. Heavy.
    Molten,
    /// The luminous drill's own light: white, lime, green.
    Plasma,
    /// Light torn off the rim of a black hole: pale violet to nothing.
    Void,
    /// A mist of acid off a spray: pale green.
    Acid,
    /// Cold white flakes from the penguin's flight plume.
    Snow,
    /// The Warden's light: white, red, a deep crimson.
    Warden,
    /// Broad red-hot flecks thrown off as a mini Warden burns apart.
    WardenFlare,
    /// The Greater Warden's light, the Warden's in violet: white, violet,
    /// a deep purple.
    Violet,
    /// Its broad violet-hot flecks, as `WardenFlare`'s.
    VioletFlare,
    /// Bright golden flecks shed by the mini Warden's lightning. They fly,
    /// bounce and quench like the other simulated sparks.
    WardenSand,
    /// Thrown off where two spells meet: white, gold, amber.
    Clash,
    /// Glitter left hanging where the penguin's spell pierces another and
    /// goes on: white, pale cyan, a deep blue. Nearly weightless, it hangs
    /// and twinkles.
    Glint,
    /// Dripped off the penguin's shield as it is drawn in the air, like
    /// sparks off a sling-ring portal: white-hot, gold, amber. Harmless.
    Ward,
    /// Flakes of the shield's fluid, lifting off it like a cold flame or
    /// splashed off where it is struck: white, pale cyan, a deep blue.
    /// Lighter than air, they rise and thin out.
    Wisp,
    /// The same off a stained shield, and its gas as it gives way: pale
    /// rose, crimson, a dark blood red.
    Taint,
    /// Motes rising off a lit nightlight (see `nightlight`): white-gold,
    /// amber, a deep ember. Lighter than air, harmless.
    Kindle,
    /// Bella's charm: glitter shed by her spells and her parasol, pale
    /// gold, rose, a deep wine. Nearly weightless, it hangs and twinkles
    /// and fades.
    Charm,
    /// Bella's chain bolts and the lightning she calls down: white, ice
    /// blue, an electric blue, a deep indigo. Quick to cool, nearly
    /// weightless: a thin crackling streak.
    Storm,
    /// Spatial magic, space torn and closing again (Bella's rifts, her
    /// dash): white, pale cyan, violet, a deep indigo. It hangs and
    /// twinkles.
    Rift,
    /// The light that bursts out of Bella as she dies: white, gold, rose,
    /// a deep violet. Lighter than air, it lifts as it fades.
    Radiance,
}

/// How a [`Glow`] looks and moves.
pub(super) struct GlowStyle {
    /// Colors from fresh to spent.
    pub ramp: [[f32; 3]; 4],
    /// Share of gravity it feels.
    pub gravity: f32,
    pub drag: f32,
    /// Share of its speed into a surface it keeps, bouncing off.
    pub bounce: f32,
    /// Heat it leaves in what it strikes.
    pub heat: f32,
    /// Light it gives its surroundings at full strength (linear).
    pub light: Vec3,
    /// How brightly it burns.
    pub brightness: f32,
    /// How much it twinkles: flaring and dimming from tick to tick (0 not
    /// at all, 1 like glitter).
    pub twinkle: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum ParticleKind {
    /// A glowing spark (see [`Glow`]). Energy is its life left in ticks; it
    /// remembers the life it started with, so it knows how far it has
    /// cooled.
    Spark(Glow),
    /// A pixel of matter a black hole tore loose, falling into it: it flies
    /// straight, touching nothing, until its energy (ticks) runs out.
    Swallowed(TerrainKind),
    Ember,
    /// Brief light at a material reaction front; color comes from the rule.
    Reaction([u8; 3]),
    /// Inert collision dust; its energy is a fade budget, never a heat source.
    Dust(TerrainKind),
    /// A cosmetic bubble: it wobbles up through liquid and pops at the
    /// surface. Energy is its remaining lifetime.
    Bubble,
    /// A tinted bubble tracing a reaction through real surrounding liquid.
    /// Its phase is its size: 0 a pinprick, 1 a bead with a rim.
    ReactionBubble([u8; 3]),
    /// A reaction front sizzling in place: a flickering point of light on
    /// the surface being eaten, froth where it sits in the liquid doing the
    /// eating. Energy is its remaining life in ticks.
    Sizzle([u8; 3]),
    /// Ambient: a firefly drifting over grass under the open sky, pulsing
    /// with its own light. Energy is its remaining lifetime.
    Firefly,
    /// Ambient: a speck of dust hanging in a cave, only visible where light
    /// catches it. Energy is its remaining lifetime.
    Drift,
    /// Ambient: a drop of water falling from a damp spot in a cave roof.
    /// It is only seen, and splashes into nothing where it lands.
    Drip,
    /// Not a mote: one real cell of this gas (flame, smoke, steam) released
    /// into open air where it is emitted, e.g. a meteor's burning wake.
    /// Energy is its temperature (0 keeps the gas's own).
    Puff(TerrainKind),
    /// A snowflake falling in the First Stop's wind (see `weather`). Its phase
    /// is how near it is (0 far and small, 1 near).
    Snowflake,
    /// A wisp of breath in the cold, or of warm air rising out of the hole
    /// in the ice: it drifts and thins out. Its phase is the life it began
    /// with.
    Vapor,
}

#[derive(Message, Clone, Copy, Debug)]
pub struct ParticleEmission {
    pub position: Vec2,
    pub velocity: Vec2,
    pub kind: ParticleKind,
    pub energy: f32,
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
pub struct Mote {
    pub position: Vec2,
    pub previous: Vec2,
    pub velocity: Vec2,
    pub kind: ParticleKind,
    pub energy: f32,
    /// Per-mote variation (ambient motes pulse and wander out of step).
    pub phase: f32,
}

impl LevelGrid {
    pub fn step_motes(&mut self, dt: f32, gravity: f32) {
        let mut motes = std::mem::take(&mut self.motes);
        let mut hiss = Vec::new();
        let mut pops = Vec::new();
        let mut splashes = Vec::new();
        self.dirty |= !motes.is_empty();
        let live = self.live_cells();
        motes.retain_mut(|mote| {
            // Motes are only for looking at: beyond the live window, where
            // nothing is seen, they are let go of. (Not snowflakes: the
            // weather keeps them to the view itself, and the sky a run opens
            // under is snowing before its world is all brought in.)
            if !matches!(mote.kind, ParticleKind::Snowflake)
                && !self.in_live(live, self.world_to_cell(mote.position))
            {
                return false;
            }
            mote.previous = mote.position;
            match mote.kind {
                ParticleKind::Bubble | ParticleKind::ReactionBubble(_) => {
                    let alive = self.step_bubble(mote, dt);
                    // A reaction bubble reaching the surface pops with a
                    // tiny spark of its own color.
                    if !alive
                        && let ParticleKind::ReactionBubble(color) = mote.kind
                        && self
                            .material_at(mote.position)
                            .is_none_or(TerrainKind::is_gas)
                    {
                        pops.push((mote.position, color, mote.phase));
                    }
                    return alive;
                }
                ParticleKind::Sizzle(_) => {
                    mote.energy -= dt * 60.0;
                    return mote.energy > 0.0;
                }
                ParticleKind::Firefly | ParticleKind::Drift => {
                    return self.step_ambient_mote(mote, dt);
                }
                ParticleKind::Snowflake => return self.step_snowflake(mote, dt),
                ParticleKind::Vapor => return self.step_vapor(mote, dt),
                ParticleKind::Spark(glow) => {
                    return self.step_spark(mote, glow, dt, gravity, &mut hiss);
                }
                ParticleKind::Swallowed(_) => {
                    mote.energy -= dt * 60.0;
                    mote.position += mote.velocity * dt;
                    return mote.energy > 0.0;
                }
                ParticleKind::Drip => {
                    mote.velocity.y -= gravity * dt;
                    let next = mote.position + mote.velocity * dt;
                    let landed = self.material_at(next).is_some_and(|kind| !kind.is_gas())
                        || self.index(self.world_to_cell(next)).is_none();
                    if landed {
                        splashes.push(mote.position);
                        return false;
                    }
                    mote.position = next;
                    return true;
                }
                _ => {}
            }
            let wet = self
                .material_at(mote.position)
                .is_some_and(TerrainKind::is_liquid);
            if wet {
                if !matches!(mote.kind, ParticleKind::Dust(_) | ParticleKind::Reaction(_)) {
                    // A spark dying in liquid hisses: heat and a bubble.
                    self.add_heat(mote.position, mote.energy * 0.18);
                    hiss.push(mote.position);
                }
                return false;
            }
            mote.energy -= dt
                * match mote.kind {
                    ParticleKind::Ember => 240.0,
                    ParticleKind::Reaction(_) => 180.0,
                    ParticleKind::Dust(_) => 180.0,
                    _ => 30.0,
                };
            if mote.energy <= 0.0 {
                return false;
            }
            mote.velocity.y -= gravity
                * dt
                * match mote.kind {
                    ParticleKind::Ember => 0.45,
                    ParticleKind::Reaction(_) => -0.05,
                    ParticleKind::Dust(_) => 0.08,
                    _ => 0.0,
                };
            let drag = match mote.kind {
                ParticleKind::Dust(_) => 6.0,
                _ => 2.0,
            };
            mote.velocity *= 1.0 / (1.0 + dt * drag);
            if let Some(hit) = self.sweep_material(
                mote.position,
                mote.position + mote.velocity * dt,
                0.7,
                true,
                true,
            ) {
                let contact = hit.position - hit.normal * (self.pixel_size * 0.5 + 0.7);
                if matches!(mote.kind, ParticleKind::Dust(_) | ParticleKind::Reaction(_)) {
                    // Dust dies against geometry rather than bouncing like hot metal.
                    return false;
                }
                if self
                    .material_at(contact)
                    .is_some_and(TerrainKind::is_liquid)
                {
                    self.add_heat(contact, mote.energy * 0.18);
                    hiss.push(contact);
                    return false;
                }
                self.add_heat(contact, mote.energy * 0.35);
                mote.position = hit.position + hit.normal * 0.05;
                let incoming = mote.velocity.dot(hit.normal);
                if incoming < 0.0 {
                    mote.velocity -= hit.normal * incoming * 1.35;
                }
                mote.energy *= 0.45;
            } else {
                mote.position += mote.velocity * dt;
            }
            true
        });
        self.motes = motes;
        for point in splashes {
            // A drip lands: a few fine droplets jump and vanish.
            let dice = self.roll(self.world_to_cell(point), 77);
            for k in 0..3 {
                let side = ((dice >> (k * 5)) % 60) as f32 - 30.0;
                self.motes.push(Mote {
                    position: point,
                    previous: point,
                    velocity: Vec2::new(side, 40.0 + ((dice >> (k * 7)) % 50) as f32),
                    kind: ParticleKind::Dust(TerrainKind::Water),
                    energy: 60.0,
                    phase: 0.0,
                });
            }
        }
        for (point, color, size) in pops {
            let cell = self.world_to_cell(point);
            if size > 0.5 || self.roll(cell, 67).is_multiple_of(2) {
                self.reaction_glint(cell, color);
            }
            if size > 0.5 {
                self.sizzle(point - Vec2::Y * self.pixel_size, color, 10.0);
            }
        }
        for point in hiss {
            if self.material_at(point).is_some_and(TerrainKind::is_liquid) {
                let dice = self.roll(self.world_to_cell(point), 61);
                self.bubble(
                    point,
                    Vec2::new((dice % 40) as f32 - 20.0, 10.0),
                    14.0 + (dice >> 8) as f32 % 18.0,
                );
            }
        }
    }

    /// A spark flies and falls as its glow says, skips off whatever is solid
    /// (losing speed into the surface and sliding along it), and settles
    /// there, glowing, until it burns out. Liquid puts it out with a hiss.
    fn step_spark(
        &mut self,
        mote: &mut Mote,
        glow: Glow,
        dt: f32,
        gravity: f32,
        hiss: &mut Vec<Vec2>,
    ) -> bool {
        mote.energy -= dt * 60.0;
        if mote.energy <= 0.0 {
            return false;
        }
        let style = glow.style();
        if glow == Glow::Void {
            // Light torn off a black hole touches nothing.
            mote.position += mote.velocity * dt;
            return true;
        }
        mote.velocity.y -= gravity * style.gravity * dt;
        mote.velocity *= 1.0 / (1.0 + dt * style.drag);
        let travel = mote.velocity * dt;
        let steps = (travel.length() / self.pixel_size).ceil().clamp(1.0, 8.0) as usize;
        let step = travel / steps as f32;
        for _ in 0..steps {
            let next = mote.position + step;
            let cell = self.world_to_cell(next);
            let Some(i) = self.index(cell) else {
                return false;
            };
            let kind = self.cells[i];
            if kind.is_none_or(TerrainKind::is_gas) {
                mote.position = next;
                continue;
            }
            if kind.is_some_and(TerrainKind::is_liquid) {
                if !matches!(glow, Glow::Acid | Glow::Snow) {
                    self.add_heat(next, 6.0 + style.heat * 0.5);
                    if self.roll(cell, 74).is_multiple_of(3) {
                        hiss.push(next);
                    }
                }
                return false;
            }
            // Solid: which way is it blocked?
            let solid =
                |grid: &Self, p: Vec2| grid.material_at(p).is_some_and(TerrainKind::is_solid);
            let across = solid(self, mote.position + Vec2::new(step.x, 0.0));
            let down = solid(self, mote.position + Vec2::new(0.0, step.y));
            let speed = mote.velocity.length();
            if across && !down {
                mote.velocity.x = -mote.velocity.x * style.bounce;
                mote.velocity.y *= 0.8;
            } else if down && !across {
                mote.velocity.y = -mote.velocity.y * style.bounce;
                // Friction along the floor: it skids, then lies there.
                mote.velocity.x *= 0.72;
                if mote.velocity.y.abs() < 12.0 {
                    mote.velocity.y = 0.0;
                }
            } else {
                mote.velocity = -mote.velocity * style.bounce;
            }
            if style.heat > 0.0 && speed > 30.0 {
                self.add_heat(next, style.heat);
            }
            // Striking hard, it loses some of its life to the blow.
            if speed > 250.0 {
                mote.energy *= 0.85;
            }
            break;
        }
        true
    }
}
