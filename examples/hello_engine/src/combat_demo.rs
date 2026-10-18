//! The combat sandbox: Rimeborn's combat (`somnium_combat`) in the editor.
//!
//! **Create > Examples > Combat Sandbox** builds an arena under one root: a
//! floor, a pond, dry grass, a snowbank, stone pillars, a draught, a fixed
//! three-quarter camera, the protagonist and two companions, and six raiders
//! (axe, spear, knife, bow, Lattice rifle, flame mage). Every fighter is an
//! ordinary entity with a **Combatant** component, so Details edits its team,
//! weapon, school, forms, pools and spells, and shows its live health, stamina,
//! mana and state during Play.
//!
//! **Play** and you are the protagonist, by mouse and keyboard
//! (`somnium_combat::input`): WASD, click to strike, right click to guard (the
//! first instant parries), Space or a tap of Shift to dodge, 1 to 0 for
//! spells (hold to shape with the mouse; the wheel edits the measure, R picks
//! which), F for the school's art, C for the Kindled Body, Q to lock on, Tab
//! to slow time. The AI fights on its own. Water freezes, grass burns, wind
//! carries the fire. The dead fall as Jolt ragdolls.
//!
//! The camera is a fixed scene camera on purpose: the isometric rig is the
//! engine lead's (Rimeborn workstream 0). Editor overlays are off in Play, so
//! everything shown is scene content: pooled meshes for missiles, walls,
//! blooms and waves; tiles for the ground's state; particle emitters for fire
//! and impacts; decals for scorch and frost; a game-UI canvas for the HUD.
use glam::{Mat4, Quat, Vec2, Vec3};
use serde_json::{Value, json};
use somnium_asset::database::AssetId;
use somnium_combat::ai::Archetype;
use somnium_combat::components::{CombatSurfaceComponent, CombatantComponent, WindVolumeComponent};
use somnium_combat::environment::{EnvironmentField, Surface, WindVolume};
use somnium_combat::input::{AimView, CombatInput, combat_map, cursor_ray, ray_plane};
use somnium_combat::spell::{Element, MeasureAxis, ROTE_SIZE, rote_name};
use somnium_combat::weapon::{ProjectileKind, WeaponKind};
use somnium_combat::{
    Action, Attitude, CombatEvent, CombatQuery, CombatRules, CombatWorld, CombatantId, DamageType,
    HitFlags, StatusKind, TeamId, WorldHit,
};
use somnium_core::{
    CameraSettingsComponent, EngineContext, EngineEvent, GameUiFrame, InputState,
    MaterialComponent, MeshKind, Name, Parent, ParticleEmitter, Transform, UiCanvas,
    WorldTransform,
};
use somnium_ecs::{Entity, World};
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::shape::ColliderShape;
use somnium_ui::message::{NodeHandle, TextMessage};
use std::collections::{BTreeMap, HashMap, VecDeque};

/// Height of a fighter's body cylinder and the head above it.
const BODY_HEIGHT: f32 = 1.5;
const BODY_WIDTH: f32 = 0.62;
const HEAD: f32 = 0.32;
/// Where a pooled entity waits when nothing uses it.
const PARKED: Vec3 = Vec3::new(0.0, -10_000.0, 0.0);
const PROJECTILES: usize = 40;
const WALLS: usize = 6;
const BLOOMS: usize = 4;
const RING_BEADS: usize = 16;
const WAVE_BEADS: usize = 10;
const WAVES: usize = 2;
const FIRES: usize = 24;
const BURSTS: usize = 12;
const DECALS: usize = 14;
const LOG_LINES: usize = 6;

fn material(name: &str) -> MaterialComponent {
    MaterialComponent {
        asset: AssetId::from_relative_path(std::path::Path::new(&format!(
            "materials/combat/{name}.sommat"
        ))),
        runtime_id: 0,
    }
}

pub fn register(r: &mut somnium_core::authoring::GameRegistration) {
    r.components.push(somnium_combat::components::register);
    r.presets.push(somnium_core::authoring::GamePreset {
        id: "hello.combat_sandbox",
        label: "Combat Sandbox",
        category: "Examples",
        schema: json!({"type":"object","properties":{"position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}},"additionalProperties":false}),
        build: spawn_sandbox,
    });
}

fn origin(args: &Value) -> Result<Vec3, String> {
    let p = Vec3::from_array(
        serde_json::from_value(args.get("position").cloned().unwrap_or(json!([0, 0.05, 0])))
            .map_err(|e| format!("{e}"))?,
    );
    if !p.is_finite() {
        return Err("Position must be finite".into());
    }
    Ok(p)
}

fn block(
    world: &mut World,
    root: Entity,
    name: &str,
    at: Vec3,
    scale: Vec3,
    kind: MeshKind,
    mat: &str,
) -> Entity {
    world.spawn((
        Name::new(name),
        Transform {
            translation: at,
            scale,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: root },
        kind,
        material(mat),
    ))
}

fn yaw(facing: Vec3) -> Quat {
    let f = Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z);
    Quat::from_rotation_y(f.x.atan2(f.z))
}

/// A fighter: a body cylinder (with the Combatant), a head and a blade marking
/// which way it faces.
fn fighter(
    world: &mut World,
    root: Entity,
    name: &str,
    feet: Vec3,
    facing: Vec3,
    combat: CombatantComponent,
) -> Vec<Entity> {
    let team = if combat.team == 0 { "party" } else { "raider" };
    let body = world.spawn((
        Name::new(name),
        Transform {
            translation: feet + Vec3::Y * BODY_HEIGHT * 0.5,
            rotation: yaw(facing),
            scale: Vec3::new(BODY_WIDTH, BODY_HEIGHT, BODY_WIDTH),
        },
        WorldTransform::identity(),
        Parent { entity: root },
        MeshKind::Cylinder,
        material(team),
        combat,
    ));
    let s = Vec3::new(BODY_WIDTH, BODY_HEIGHT, BODY_WIDTH);
    let head = world.spawn((
        Name::new(&format!("{name} head")),
        Transform {
            translation: Vec3::new(0.0, (BODY_HEIGHT * 0.5 + HEAD * 0.5) / BODY_HEIGHT, 0.0),
            scale: Vec3::splat(HEAD) / s,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: body },
        MeshKind::Sphere,
        material("skin"),
    ));
    let blade = world.spawn((
        Name::new(&format!("{name} weapon")),
        Transform {
            translation: Vec3::new(0.25, 0.12, 0.45) / s,
            scale: Vec3::new(0.06, 0.06, 0.9) / s,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: body },
        MeshKind::Cube,
        material("dark"),
    ));
    vec![body, head, blade]
}

fn raider(archetype: Archetype, weapon: WeaponKind) -> CombatantComponent {
    CombatantComponent {
        team: 1,
        archetype: archetype as u32,
        weapon: weapon as u32,
        ..CombatantComponent::default()
    }
}

fn spawn_sandbox(world: &mut World, args: &Value) -> Result<Vec<Entity>, String> {
    let at = origin(args)?;
    let root = world.spawn((
        Name::new("Combat Sandbox"),
        Transform::from_translation(at),
        WorldTransform::identity(),
    ));
    let mut out = vec![root];

    let floor = block(
        world,
        root,
        "Arena floor",
        Vec3::new(0.0, -0.1, 0.0),
        Vec3::new(40.0, 0.2, 40.0),
        MeshKind::Cube,
        "floor",
    );
    let _ = world.insert_component(
        floor,
        CombatSurfaceComponent {
            surface: Surface::Bare as u32,
            round: false,
            solid: true,
        },
    );
    out.push(floor);
    for (name, pos, scale, kind, mat, surface, round) in [
        (
            "Pond",
            Vec3::new(-8.0, 0.012, 1.0),
            Vec3::new(7.0, 0.02, 7.0),
            MeshKind::Cylinder,
            "water",
            Surface::Water,
            true,
        ),
        (
            "Dry grass",
            Vec3::new(8.0, 0.012, 2.0),
            Vec3::new(10.0, 0.02, 9.0),
            MeshKind::Cube,
            "grass",
            Surface::Grass,
            false,
        ),
        (
            "Snowbank",
            Vec3::new(-9.0, 0.012, -9.0),
            Vec3::new(6.0, 0.02, 5.0),
            MeshKind::Cube,
            "snow",
            Surface::Snow,
            false,
        ),
    ] {
        let e = block(world, root, name, pos, scale, kind, mat);
        let _ = world.insert_component(
            e,
            CombatSurfaceComponent {
                surface: surface as u32,
                round,
                solid: false,
            },
        );
        out.push(e);
    }
    for (i, pos) in [
        Vec3::new(-3.0, 1.5, 2.0),
        Vec3::new(3.5, 1.5, 0.0),
        Vec3::new(0.5, 1.5, 11.0),
    ]
    .into_iter()
    .enumerate()
    {
        let e = block(
            world,
            root,
            &format!("Stone pillar {}", i + 1),
            pos,
            Vec3::new(1.2, 3.0, 1.2),
            MeshKind::Cube,
            "stone",
        );
        let _ = world.insert_component(
            e,
            CombatSurfaceComponent {
                surface: Surface::Stone as u32,
                round: false,
                solid: true,
            },
        );
        out.push(e);
    }
    out.push(world.spawn((
        Name::new("Draught"),
        Transform::from_translation(Vec3::new(4.0, 1.0, 3.0)),
        WorldTransform::identity(),
        Parent { entity: root },
        WindVolumeComponent {
            direction: Vec3::X,
            strength: 7.0,
            radius: 9.0,
        },
    )));
    // A three-quarter view over the field; the isometric rig replaces it.
    let eye = Vec3::new(0.0, 26.0, -26.0);
    let camera_world = Mat4::look_at_rh(eye, Vec3::new(0.0, 0.0, 0.5), Vec3::Y).inverse();
    let (_, rotation, _) = camera_world.to_scale_rotation_translation();
    out.push(world.spawn((
        Name::new("Combat Camera"),
        Transform {
            translation: eye,
            rotation,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: root },
        CameraSettingsComponent::default(),
    )));

    let party = Vec3::Z;
    let foe = -Vec3::Z;
    out.extend(fighter(
        world,
        root,
        "Protagonist",
        Vec3::new(0.0, 0.0, -8.0),
        party,
        CombatantComponent::protagonist(),
    ));
    // Tyffir's mount, saddled beside him: E climbs on.
    out.push(world.spawn((
        Name::new("Frost-elk"),
        Transform {
            translation: Vec3::new(3.2, 0.8, -7.5),
            rotation: yaw(party),
            scale: Vec3::new(0.9, 1.6, 2.3),
        },
        WorldTransform::identity(),
        Parent { entity: root },
        MeshKind::Cube,
        material("hide"),
        CombatantComponent {
            team: 0,
            mount: 2,
            max_health: 220.0,
            poise: 90.0,
            ..CombatantComponent::default()
        },
    )));
    out.extend(fighter(
        world,
        root,
        "Companion (spear)",
        Vec3::new(-2.2, 0.0, -9.5),
        party,
        CombatantComponent {
            team: 0,
            archetype: Archetype::Companion as u32,
            weapon: WeaponKind::Spear as u32,
            max_health: 130.0,
            ..CombatantComponent::default()
        },
    ));
    out.extend(fighter(
        world,
        root,
        "Companion (mender)",
        Vec3::new(2.2, 0.0, -9.5),
        party,
        CombatantComponent {
            team: 0,
            archetype: Archetype::Companion as u32,
            weapon: WeaponKind::Unarmed as u32,
            capacity: 220.0,
            spells: "close_wound,water_bolt".into(),
            ..CombatantComponent::default()
        },
    ));
    for (name, pos, mut c) in [
        (
            "Raider (axe)",
            Vec3::new(-3.0, 0.0, 7.0),
            raider(Archetype::Brute, WeaponKind::Axe),
        ),
        (
            "Raider (spear)",
            Vec3::new(0.0, 0.0, 8.0),
            raider(Archetype::Spearman, WeaponKind::Spear),
        ),
        (
            "Raider (knife)",
            Vec3::new(3.5, 0.0, 6.5),
            raider(Archetype::Skirmisher, WeaponKind::Knife),
        ),
        (
            "Raider (bow)",
            Vec3::new(-7.0, 0.0, 13.0),
            raider(Archetype::Archer, WeaponKind::Bow),
        ),
        (
            "Raider (Lattice rifle)",
            Vec3::new(7.0, 0.0, 14.0),
            raider(Archetype::Rifleman, WeaponKind::LatticeRifle),
        ),
        (
            "Raider (flame mage)",
            Vec3::new(0.0, 0.0, 15.0),
            raider(Archetype::Caster, WeaponKind::Unarmed),
        ),
    ] {
        match c.weapon_kind() {
            WeaponKind::Axe => {
                c.max_health = 140.0;
                c.poise = 60.0;
                c.armor = 4.0;
            }
            WeaponKind::Knife => c.speed = 4.3,
            WeaponKind::Unarmed => {
                c.capacity = 160.0;
                c.rank = 2;
                c.spells = "ember_bolt,fireburst,close_wound".into();
            }
            _ => {}
        }
        out.extend(fighter(world, root, name, pos, foe, c));
    }
    Ok(out)
}

/// Build the sandbox at the origin outside the Create menu (the capture hook).
pub fn spawn_at_startup(world: &mut World) -> Result<usize, String> {
    spawn_sandbox(world, &json!({})).map(|entities| entities.len())
}

/// Whether the scene has a player-controlled fighter, so Play is the
/// sandbox's (hello_engine then spawns no first-person character).
pub fn claims_play(world: &World) -> bool {
    world
        .iter_with::<CombatantComponent>()
        .any(|(_, c)| c.enabled && c.player)
}

/// The physics world as combat sees it: a thin capsule swept through Jolt.
struct JoltQuery<'a>(&'a somnium_physics::world::PhysicsWorld);

impl CombatQuery for JoltQuery<'_> {
    fn sweep(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit> {
        let hit = self
            .0
            .cast_capsule(somnium_physics::animation::CapsuleSweep {
                position: from,
                rotation: Quat::IDENTITY,
                displacement: to - from,
                half_height: 0.001,
                radius: radius.max(0.01),
                ignore_body: None,
            })
            .ok()??;
        Some(WorldHit {
            fraction: hit.fraction,
            normal: hit.normal,
        })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Hidden,
    Shown(Vec3, Quat, Vec3),
}

/// Pooled entities for what comes and goes.
#[derive(Default)]
struct Pools {
    projectiles: Vec<(Entity, &'static str)>,
    walls: Vec<(Entity, &'static str)>,
    blooms: Vec<(Entity, &'static str)>,
    /// Which bloom a ring's beads belong to has nothing to do with order here:
    /// ring `i` is beads `i * RING_BEADS ..`.
    beads: Vec<Entity>,
    fires: Vec<(Entity, bool)>,
    bursts: Vec<Entity>,
    decals: Vec<Entity>,
    next_burst: usize,
    next_decal: usize,
    looks: HashMap<Entity, Look>,
}

struct Run {
    world: CombatWorld,
    /// Parent of every entity Play spawns (pools, ground tiles).
    effects: Option<Entity>,
    bodies: BTreeMap<CombatantId, Entity>,
    /// Each body's world size as authored (a fighter's cylinder, a mount's box).
    dims: BTreeMap<CombatantId, Vec3>,
    player: Option<CombatantId>,
    camera: Option<Entity>,
    restore: Vec<(Entity, Transform)>,
    colliders: Vec<BodyId>,
    ragdolls: BTreeMap<CombatantId, somnium_physics::animation::RagdollId>,
    /// Field tile (2 x 2 cells) to its entity and the surface it shows.
    tiles: BTreeMap<(u32, u32), (Entity, Surface)>,
    floor: f32,
    pools: Pools,
    spawned: Vec<Entity>,
    log: VecDeque<String>,
    readouts: HashMap<Entity, (i32, i32, i32, &'static str)>,
}

struct Hud {
    canvas: UiCanvas,
    vitals: NodeHandle,
    hotbar: NodeHandle,
    log: NodeHandle,
    shown: [String; 3],
    /// The viewport rectangle (logical x, y, w, h) the panels were laid out in.
    rect: (u32, u32, u32, u32),
}

impl Hud {
    /// Panels placed inside the editor's viewport: in Play the editor keeps
    /// its docks, and the game canvas covers the whole window.
    fn new(rect: (u32, u32, u32, u32)) -> Self {
        use somnium_ui::types::{HorizontalAlignment as H, Thickness, VerticalAlignment as V};
        use somnium_ui::widget::WidgetBuilder;
        use somnium_ui::widgets::{border::BorderBuilder, text::TextBuilder};
        let (vx, vy, vw, vh) = (rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32);
        let mut canvas = UiCanvas::new(1280.0, 720.0);
        let font = canvas.font_id();
        let ui = canvas.ui_mut();
        let root = ui.root();
        let panel = |ui: &mut somnium_ui::ui::UserInterface, x: f32, y: f32, w: f32, h: f32| {
            let p = BorderBuilder::new(
                WidgetBuilder::new()
                    .with_width(w.max(40.0))
                    .with_height(h)
                    .with_horizontal_alignment(H::Left)
                    .with_vertical_alignment(V::Top)
                    .with_margin(Thickness {
                        left: x.max(0.0),
                        top: y.max(0.0),
                        right: 0.0,
                        bottom: 0.0,
                    })
                    .with_background([12, 14, 18, 200])
                    .with_foreground(somnium_ui::theme::BORDER_DARK),
            )
            .with_stroke_thickness(Thickness::uniform(1.0))
            .build();
            let p = ui.add_node(p, root);
            let text = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::uniform(8.0)))
                .with_text("")
                .with_font_id(font)
                .with_font_size(13.0)
                .with_wrap(true)
                .with_color(somnium_ui::theme::TEXT_PRIMARY)
                .build();
            ui.add_node(text, p)
        };
        let gap = 10.0;
        let vitals_w = 330.0f32.min(vw * 0.45);
        let vitals = panel(ui, vx + gap, vy + vh - gap - 112.0, vitals_w, 112.0);
        let hotbar_w = (vw - vitals_w - 3.0 * gap).max(120.0);
        let hotbar = panel(
            ui,
            vx + vitals_w + 2.0 * gap,
            vy + vh - gap - 112.0,
            hotbar_w,
            112.0,
        );
        let log_w = 330.0f32.min(vw * 0.5);
        // Below the viewport's own toolbar row.
        let log = panel(ui, vx + vw - gap - log_w, vy + 48.0, log_w, 140.0);
        Self {
            canvas,
            vitals,
            hotbar,
            log,
            shown: Default::default(),
            rect,
        }
    }

    fn set(&mut self, which: usize, text: String) {
        if self.shown[which] != text {
            let node = [self.vitals, self.hotbar, self.log][which];
            self.canvas
                .ui_mut()
                .send(TextMessage::set_text(node, text.clone()));
            self.shown[which] = text;
        }
    }
}

/// The sandbox's runtime.
pub struct CombatDemo {
    run: Option<Run>,
    input: somnium_input::InputSystem,
    reader: CombatInput,
    cursor: Option<Vec2>,
    hud: Option<Hud>,
}

impl Default for CombatDemo {
    fn default() -> Self {
        let mut input = somnium_input::InputSystem::new();
        input.add_map(combat_map());
        Self {
            run: None,
            input,
            reader: CombatInput::new(),
            cursor: None,
            hud: None,
        }
    }
}

fn world_matrix(world: &World, entity: Entity) -> Mat4 {
    world
        .get::<WorldTransform>(entity)
        .map(|w| w.0)
        .unwrap_or_else(|| {
            world
                .get::<Transform>(entity)
                .map_or(Mat4::IDENTITY, Transform::to_matrix)
        })
}

fn surface_material(surface: Surface) -> &'static str {
    match surface {
        Surface::Bare => "floor",
        Surface::Grass => "grass",
        Surface::Water => "water",
        Surface::Ice => "ice",
        Surface::Snow => "snow",
        Surface::Burning => "fire",
        Surface::Ash => "ash",
        Surface::Stone => "stone",
    }
}

fn element_material(element: Element) -> &'static str {
    match element {
        Element::Flame => "fire",
        Element::Frost => "frost",
        Element::Water => "water",
        Element::Gale => "gale",
        Element::Stone => "stone",
        Element::Mending => "mending",
        Element::Force => "force",
    }
}

fn element_of(kind: DamageType) -> Option<Element> {
    Some(match kind {
        DamageType::Frost => Element::Frost,
        DamageType::Flame => Element::Flame,
        DamageType::Gale => Element::Gale,
        DamageType::Stone => Element::Stone,
        DamageType::Mana => Element::Force,
        _ => return None,
    })
}

fn burst_colour(element: Option<Element>, heal: bool) -> ([f32; 4], [f32; 4]) {
    if heal {
        return ([0.4, 1.0, 0.5, 1.0], [0.1, 0.5, 0.2, 0.0]);
    }
    match element {
        Some(Element::Flame) => ([1.0, 0.55, 0.12, 1.0], [0.4, 0.05, 0.0, 0.0]),
        Some(Element::Frost) => ([0.75, 0.9, 1.0, 1.0], [0.3, 0.5, 0.8, 0.0]),
        Some(Element::Water) => ([0.4, 0.6, 0.9, 1.0], [0.1, 0.2, 0.4, 0.0]),
        Some(Element::Gale) => ([0.85, 0.95, 0.9, 0.8], [0.6, 0.7, 0.7, 0.0]),
        Some(Element::Stone) => ([0.5, 0.45, 0.4, 1.0], [0.2, 0.18, 0.15, 0.0]),
        Some(Element::Force) => ([0.7, 0.55, 1.0, 1.0], [0.3, 0.2, 0.6, 0.0]),
        Some(Element::Mending) => ([0.4, 1.0, 0.5, 1.0], [0.1, 0.5, 0.2, 0.0]),
        None => ([0.7, 0.04, 0.03, 1.0], [0.25, 0.0, 0.0, 0.0]),
    }
}

impl CombatDemo {
    /// Feed the combat map from engine events.
    pub fn event(&mut self, event: &EngineEvent) {
        let devices = self.input.devices_mut();
        match event {
            EngineEvent::KeyInput { key, state } => {
                if let Some(name) = somnium_input::device::key_name(*key) {
                    devices.set_key(name, *state == InputState::Pressed);
                }
            }
            EngineEvent::MouseButton { button, state } => {
                let name = match button {
                    somnium_core::MouseButton::Left => "leftbutton",
                    somnium_core::MouseButton::Right => "rightbutton",
                    somnium_core::MouseButton::Middle => "middlebutton",
                    _ => return,
                };
                devices.set_mouse_button(name, *state == InputState::Pressed);
            }
            EngineEvent::MouseWheel { delta_y } => devices.add_scroll(*delta_y as f32),
            EngineEvent::CursorMoved { x, y } => {
                self.cursor = Some(Vec2::new(*x as f32, *y as f32))
            }
            EngineEvent::WindowFocused(false) => devices.release_all(),
            _ => {}
        }
    }

    /// The sandbox camera's world matrix while a fight runs.
    pub fn view(&self, world: &World) -> Option<Mat4> {
        self.run.as_ref()?.camera.map(|c| world_matrix(world, c))
    }

    /// Build the fight from the scene's authored fighters, ground and wind.
    pub fn begin(&mut self, ctx: &mut EngineContext) {
        somnium_core::propagate_transforms(ctx.world);
        if let Some(run) = build(ctx.world) {
            self.run = Some(run);
            let run = self.run.as_mut().expect("just built");
            run.colliders = colliders(ctx);
            spawn_pools(ctx.world, run);
            group_under_effects(ctx.world, run);
            run.log.push_back(
                "Fight! WASD, click, right click, Space, 1-0, F, C, Q, Tab; E rides the elk".into(),
            );
        }
    }

    /// Put everything back as authored.
    pub fn end(&mut self, ctx: &mut EngineContext) {
        let Some(run) = self.run.take() else {
            return;
        };
        for (entity, transform) in run.restore {
            if let Some(t) = ctx.world.get_mut::<Transform>(entity) {
                *t = transform;
            }
        }
        for entity in run.spawned {
            ctx.world.despawn(entity);
        }
        for (_, ragdoll) in run.ragdolls {
            let _ = ctx.physics.destroy_ragdoll(ragdoll);
        }
        for body in run.colliders {
            ctx.physics.destroy_body(body);
        }
        self.input.devices_mut().release_all();
    }

    /// The player's command for the coming fixed steps (once a frame).
    pub fn update(&mut self, ctx: &mut EngineContext) {
        let Some(run) = &mut self.run else {
            return;
        };
        self.input.update(ctx.dt());
        let Some(player) = run.player else {
            return;
        };
        let camera = run.camera.map(|c| world_matrix(ctx.world, c));
        let feet = run
            .world
            .combatant(player)
            .map_or(run.floor, |c| c.position.y);
        let aim = camera.zip(self.cursor).and_then(|(camera, cursor)| {
            let (view, _) = somnium_core::camera_view_from_world(camera);
            let (x, y, w, h) = ctx.ui.viewport_physical_rect(1.0);
            let size = Vec2::new(w as f32, h as f32);
            let projection =
                Mat4::perspective_rh(45f32.to_radians(), size.x / size.y.max(1.0), 0.1, 1000.0);
            let (o, d) = cursor_ray(
                cursor - Vec2::new(x as f32, y as f32),
                size,
                view,
                projection,
            )?;
            ray_plane(o, d, feet)
        });
        let forward = camera.map_or(Vec3::Z, |m| -m.z_axis.truncate());
        let cmd = self.reader.command(
            &self.input,
            ctx.dt(),
            AimView {
                aim,
                camera_forward: forward,
            },
        );
        run.world.command(player, cmd);
    }

    /// One fixed step of the fight, then everything it changed onto the scene.
    pub fn fixed(&mut self, ctx: &mut EngineContext) {
        let Some(run) = &mut self.run else {
            return;
        };
        let dt = ctx.simulation.fixed_delta_seconds;
        let nav = &ctx.navigation.cells.world;
        let nav = nav.cells().next().is_some().then_some(nav);
        run.world.step(dt, &JoltQuery(ctx.physics), nav);
        let events = run.world.drain_events();
        for event in &events {
            react(ctx, run, event);
        }
        write_scene(ctx, run);
    }

    /// HUD text (once a frame, before the UI is drawn).
    pub fn render(&mut self, ctx: &mut EngineContext) {
        let Some(run) = &self.run else {
            return;
        };
        let rect = ctx.ui.viewport_physical_rect(1.0);
        if self.hud.as_ref().is_none_or(|h| h.rect != rect) {
            self.hud = Some(Hud::new(rect));
        }
        let hud = self.hud.as_mut().expect("just built");
        let (vitals, hotbar) = player_lines(run);
        hud.set(0, vitals);
        hud.set(1, hotbar);
        hud.set(2, run.log.iter().cloned().collect::<Vec<_>>().join("\n"));
    }

    /// Draw the HUD.
    pub fn draw_ui(&mut self, frame: &mut GameUiFrame) {
        if self.run.is_some()
            && let Some(hud) = &mut self.hud
        {
            frame.draw(&mut hud.canvas);
        }
    }

    /// For MCP and automation.
    pub fn state(&self) -> Value {
        let Some(run) = &self.run else {
            return json!({"running": false});
        };
        let fighters: Vec<Value> = run
            .world
            .combatants()
            .map(|c| {
                json!({
                    "name": c.name,
                    "team": c.team.0,
                    "health": (c.vitals.health.current * 10.0).round() / 10.0,
                    "state": c.action.label(),
                    "doing": c.brain.as_ref().map(|b| b.mind.doing),
                    "position": c.position.to_array(),
                })
            })
            .collect();
        json!({
            "running": true,
            "time": run.world.time(),
            "fighters": fighters,
            "log": run.log,
        })
    }
}

/// The combat world from the scene: fighters, teams, ground, wind.
fn build(world: &World) -> Option<Run> {
    let fighters: Vec<(Entity, CombatantComponent)> = world
        .iter_with::<CombatantComponent>()
        .filter(|(_, c)| c.enabled)
        .map(|(e, c)| (e, c.clone()))
        .collect();
    if fighters.is_empty() {
        return None;
    }
    let mut combat = CombatWorld::new(CombatRules {
        integrate_motion: true,
        seed: 0x00C0_FFEE,
        ..CombatRules::default()
    });
    let mut teams: Vec<u32> = fighters.iter().map(|(_, c)| c.team).collect();
    teams.sort_unstable();
    teams.dedup();
    for &t in &teams {
        if t != 0 {
            combat
                .relations
                .set(TeamId(0), TeamId(t as u16), Attitude::Hostile);
        }
    }
    let mut bodies = BTreeMap::new();
    let mut dims = BTreeMap::new();
    let mut restore = Vec::new();
    let mut player = None;
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    let mut floor = f32::NEG_INFINITY;
    for (entity, c) in &fighters {
        let m = world_matrix(world, *entity);
        let feet = m.transform_point3(Vec3::new(0.0, -0.5, 0.0));
        let facing = m.transform_vector3(Vec3::Z);
        let name = world.get::<Name>(*entity).map_or("Fighter", |n| n.as_str());
        let host = (u64::from(entity.generation()) << 32) | u64::from(entity.index());
        let id = combat.spawn(c.desc(name, feet, facing, host));
        if c.player && player.is_none() {
            player = Some(id);
        }
        bodies.insert(id, *entity);
        dims.insert(id, m.to_scale_rotation_translation().0.abs());
        if let Some(t) = world.get::<Transform>(*entity) {
            restore.push((*entity, *t));
        }
        min = min.min(Vec2::new(feet.x, feet.z));
        max = max.max(Vec2::new(feet.x, feet.z));
        floor = floor.max(feet.y);
    }
    if let Some(p) = player {
        let companions: Vec<CombatantId> = combat
            .combatants()
            .filter(|c| {
                c.brain
                    .as_ref()
                    .is_some_and(|b| b.mind.archetype == Archetype::Companion)
            })
            .map(|c| c.id)
            .collect();
        for id in companions {
            if let Some(brain) = combat.combatant_mut(id).and_then(|c| c.brain.as_mut()) {
                brain.mind.leader = Some(p);
            }
        }
    }
    // The ground: every authored surface's footprint (axis-aligned), plus wind.
    let surfaces: Vec<(Mat4, CombatSurfaceComponent)> = world
        .iter_with::<CombatSurfaceComponent>()
        .map(|(e, s)| (world_matrix(world, e), *s))
        .collect();
    for (m, s) in &surfaces {
        if s.surface() == Surface::Bare {
            continue;
        }
        let (scale, _, centre) = m.to_scale_rotation_translation();
        let half = Vec2::new(scale.x, scale.z).abs() * 0.5;
        min = min.min(Vec2::new(centre.x, centre.z) - half);
        max = max.max(Vec2::new(centre.x, centre.z) + half);
    }
    let pad = Vec2::splat(12.0);
    combat.environment = EnvironmentField::new(min - pad, max + pad, 0.5);
    if let Some(field) = &mut combat.environment {
        for (m, s) in &surfaces {
            if s.surface() == Surface::Bare {
                continue;
            }
            let (scale, _, centre) = m.to_scale_rotation_translation();
            let c = Vec2::new(centre.x, centre.z);
            let half = Vec2::new(scale.x, scale.z).abs() * 0.5;
            if s.round {
                field.paint_circle(c, half.max_element(), s.surface());
            } else {
                field.paint_rect(c - half, c + half, s.surface());
            }
        }
        for (e, w) in world.iter_with::<WindVolumeComponent>() {
            let centre = world_matrix(world, e).transform_point3(Vec3::ZERO);
            field.winds.push(WindVolume {
                center: centre,
                radius: w.radius.max(0.5),
                velocity: Vec3::new(w.direction.x, 0.0, w.direction.z).normalize_or_zero()
                    * w.strength,
                time_left: f32::INFINITY,
            });
        }
    }
    let camera = world
        .iter_with::<CameraSettingsComponent>()
        .map(|(e, _)| e)
        .find(|e| {
            world
                .get::<Name>(*e)
                .is_some_and(|n| n.as_str() == "Combat Camera")
        });
    Some(Run {
        world: combat,
        effects: None,
        bodies,
        dims,
        player,
        camera,
        restore,
        colliders: Vec::new(),
        ragdolls: BTreeMap::new(),
        tiles: BTreeMap::new(),
        floor: if floor.is_finite() { floor } else { 0.0 },
        pools: Pools::default(),
        spawned: Vec::new(),
        log: VecDeque::new(),
        readouts: HashMap::new(),
    })
}

/// Static Jolt boxes for solid surfaces (the floor, pillars), so ragdolls
/// land and the combat query sees them.
fn colliders(ctx: &mut EngineContext) -> Vec<BodyId> {
    let solids: Vec<Mat4> = ctx
        .world
        .iter_with::<CombatSurfaceComponent>()
        .filter(|(_, s)| s.solid)
        .map(|(e, _)| world_matrix(ctx.world, e))
        .collect();
    solids
        .into_iter()
        .map(|m| {
            let (scale, rotation, translation) = m.to_scale_rotation_translation();
            ctx.physics.create_body(RigidBodyDescriptor {
                shape: ColliderShape::Box {
                    half_extents: (scale.abs() * 0.5).max(Vec3::splat(0.01)),
                },
                position: translation,
                rotation,
                motion_type: MotionType::Static,
                object_layer: somnium_physics::layer::LAYER_NON_MOVING,
                ..RigidBodyDescriptor::default()
            })
        })
        .filter(|b| b.is_valid())
        .collect()
}

fn pooled(world: &mut World, run: &mut Run, name: String, kind: MeshKind, mat: &str) -> Entity {
    let e = world.spawn((
        Name::new(&name),
        Transform::from_translation(PARKED),
        WorldTransform::identity(),
        kind,
        material(mat),
    ));
    run.spawned.push(e);
    run.pools.looks.insert(e, Look::Hidden);
    e
}

fn spawn_pools(world: &mut World, run: &mut Run) {
    let effects = world.spawn((
        Name::new("Combat (runtime)"),
        Transform::default(),
        WorldTransform::identity(),
    ));
    run.spawned.push(effects);
    run.effects = Some(effects);
    for i in 0..PROJECTILES {
        let e = pooled(
            world,
            run,
            format!("Combat missile {i}"),
            MeshKind::Sphere,
            "dark",
        );
        run.pools.projectiles.push((e, "dark"));
    }
    for i in 0..WALLS {
        let e = pooled(
            world,
            run,
            format!("Combat wall {i}"),
            MeshKind::Cube,
            "stone",
        );
        run.pools.walls.push((e, "stone"));
    }
    for i in 0..BLOOMS * RING_BEADS {
        let e = pooled(
            world,
            run,
            format!("Combat bloom {i}"),
            MeshKind::Sphere,
            "fire",
        );
        run.pools.blooms.push((e, "fire"));
    }
    for i in 0..WAVE_BEADS * WAVES {
        let e = pooled(
            world,
            run,
            format!("Combat wave {i}"),
            MeshKind::Sphere,
            "gale",
        );
        run.pools.beads.push(e);
    }
    for i in 0..FIRES {
        let e = world.spawn((
            Name::new(&format!("Combat fire {i}")),
            Transform::from_translation(PARKED),
            WorldTransform::identity(),
            ParticleEmitter {
                enabled: false,
                additive: true,
                max_particles: 40,
                spawn_rate: 16.0,
                lifetime: 0.8,
                initial_speed: 0.9,
                spread_angle: 0.35,
                size_start: 0.45,
                size_end: 0.08,
                gravity: -1.6,
                spawn_extents: [0.35, 0.0, 0.35],
                color_over_life: somnium_ecs::curve::Gradient::ramp(
                    [1.0, 0.55, 0.12, 1.0],
                    [0.6, 0.05, 0.0, 0.0],
                ),
                ..ParticleEmitter::default()
            },
        ));
        run.spawned.push(e);
        run.pools.fires.push((e, false));
    }
    for i in 0..BURSTS {
        let e = world.spawn((
            Name::new(&format!("Combat impact {i}")),
            Transform::from_translation(PARKED),
            WorldTransform::identity(),
            ParticleEmitter {
                additive: true,
                max_particles: 48,
                spawn_rate: 0.0,
                lifetime: 0.45,
                initial_speed: 3.0,
                spread_angle: 1.4,
                size_start: 0.16,
                size_end: 0.02,
                gravity: 4.0,
                ..ParticleEmitter::default()
            },
        ));
        run.spawned.push(e);
        run.pools.bursts.push(e);
    }
    for i in 0..DECALS {
        let e = world.spawn((
            Name::new(&format!("Combat mark {i}")),
            Transform::from_translation(PARKED),
            WorldTransform::identity(),
            somnium_core::decal::DecalComponent {
                multiply: true,
                roughness: 0.9,
                ..Default::default()
            },
            material("ash"),
        ));
        run.spawned.push(e);
        run.pools.decals.push(e);
    }
    // Ground tiles: one per two-by-two cells that is not bare.
    let Some(field) = &run.world.environment else {
        return;
    };
    let (w, d) = field.dimensions();
    let cell = field.cell_size();
    let mut tiles = Vec::new();
    for tz in 0..d.div_ceil(2) {
        for tx in 0..w.div_ceil(2) {
            let s = tile_surface(field, tx, tz);
            if s != Surface::Bare {
                tiles.push((tx, tz, s));
            }
        }
    }
    let floor = run.floor;
    for (tx, tz, s) in tiles {
        let c = field.center(tx * 2, tz * 2) + Vec2::splat(cell * 0.5);
        let e = world.spawn((
            Name::new(&format!("Combat ground {tx},{tz}")),
            Transform {
                translation: Vec3::new(c.x, floor + 0.03, c.y),
                scale: Vec3::new(cell * 2.0 - 0.03, 0.02, cell * 2.0 - 0.03),
                ..Default::default()
            },
            WorldTransform::identity(),
            MeshKind::Cube,
            material(surface_material(s)),
        ));
        run.spawned.push(e);
        run.tiles.insert((tx, tz), (e, s));
    }
}

/// Hang every runtime entity under the one root (at the origin, so local is world).
fn group_under_effects(world: &mut World, run: &Run) {
    let Some(root) = run.effects else {
        return;
    };
    for &e in &run.spawned {
        if e != root {
            let _ = world.insert_component(e, Parent { entity: root });
        }
    }
}

/// What a two-by-two tile shows: the most telling of its cells.
fn tile_surface(field: &EnvironmentField, tx: u32, tz: u32) -> Surface {
    let rank = |s: Surface| match s {
        Surface::Burning => 7,
        Surface::Ice => 6,
        Surface::Ash => 5,
        Surface::Water => 4,
        Surface::Snow => 3,
        Surface::Grass => 2,
        Surface::Stone => 1,
        Surface::Bare => 0,
    };
    let (w, d) = field.dimensions();
    let mut best = Surface::Bare;
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let (x, z) = (tx * 2 + dx, tz * 2 + dz);
        if x >= w || z >= d {
            continue;
        }
        let c = field.center(x, z);
        let s = field.surface_at(Vec3::new(c.x, 0.0, c.y));
        if rank(s) > rank(best) {
            best = s;
        }
    }
    best
}

fn name_of(run: &Run, id: CombatantId) -> String {
    run.world
        .combatant(id)
        .map_or_else(|| "someone".into(), |c| c.name.clone())
}

fn log(run: &mut Run, line: String) {
    run.log.push_back(line);
    while run.log.len() > LOG_LINES {
        run.log.pop_front();
    }
}

fn burst(
    world: &mut World,
    run: &mut Run,
    at: Vec3,
    element: Option<Element>,
    heal: bool,
    count: u32,
) {
    let Some(&e) = run.pools.bursts.get(run.pools.next_burst % BURSTS) else {
        return;
    };
    run.pools.next_burst += 1;
    let (a, b) = burst_colour(element, heal);
    if let Some(t) = world.get_mut::<Transform>(e) {
        t.translation = at;
    }
    if let Some(p) = world.get_mut::<ParticleEmitter>(e) {
        p.color_over_life = somnium_ecs::curve::Gradient::ramp(a, b);
        p.burst = count;
    }
}

fn mark(world: &mut World, run: &mut Run, at: Vec3, size: f32, mat: &str) {
    let Some(&e) = run.pools.decals.get(run.pools.next_decal % DECALS) else {
        return;
    };
    run.pools.next_decal += 1;
    let mut t = somnium_core::decal::placement(at, Vec3::Y);
    t.scale = Vec3::new(size, 0.5, size);
    if let Some(slot) = world.get_mut::<Transform>(e) {
        *slot = t;
    }
    if let Some(m) = world.get_mut::<MaterialComponent>(e) {
        *m = material(mat);
    }
}

/// Turn one event into presentation: ragdolls, impacts, marks, the log.
fn react(ctx: &mut EngineContext, run: &mut Run, event: &CombatEvent) {
    match event {
        CombatEvent::Damaged {
            target,
            source,
            amount,
            kind,
            point,
            critical,
            flags,
        } => {
            if flags.contains(HitFlags::TICK) {
                return;
            }
            burst(
                ctx.world,
                run,
                *point,
                element_of(*kind),
                false,
                if *critical { 40 } else { 18 },
            );
            let by = source.map_or_else(|| "the ground".to_string(), |s| name_of(run, s));
            let crit = if *critical { " critical" } else { "" };
            log(
                run,
                format!(
                    "{by} > {}: {:.0} {}{crit}",
                    name_of(run, *target),
                    amount,
                    kind.name().to_lowercase()
                ),
            );
        }
        CombatEvent::Healed { target, amount, .. } => {
            if let Some(c) = run.world.combatant(*target) {
                let at = c.chest();
                burst(ctx.world, run, at, None, true, 24);
            }
            log(
                run,
                format!("{} healed {:.0}", name_of(run, *target), amount),
            );
        }
        CombatEvent::Parried {
            defender,
            attacker,
            point,
            perfect,
        } => {
            burst(ctx.world, run, *point, Some(Element::Force), false, 30);
            let noon = if *perfect {
                " at the instant (Noon)"
            } else {
                ""
            };
            log(
                run,
                format!(
                    "{} parried {}{noon}",
                    name_of(run, *defender),
                    name_of(run, *attacker)
                ),
            );
        }
        CombatEvent::Blocked { target, point, .. } => {
            burst(ctx.world, run, *point, Some(Element::Stone), false, 10);
            let _ = target;
        }
        CombatEvent::GuardBroken { target } => {
            log(run, format!("{}'s guard broke", name_of(run, *target)))
        }
        CombatEvent::Reflected { defender, .. } => log(
            run,
            format!("{} turned a bolt back", name_of(run, *defender)),
        ),
        CombatEvent::Shattered { target } => {
            log(run, format!("{} shattered", name_of(run, *target)))
        }
        CombatEvent::Feinted { attacker } => {
            log(run, format!("{} feinted", name_of(run, *attacker)))
        }
        CombatEvent::SpellReleased {
            caster, spell, at, ..
        } => {
            let name = run
                .world
                .spell(spell)
                .map_or(spell.clone(), |s| s.name.clone());
            log(run, format!("{}: {name}", name_of(run, *caster)));
            let _ = at;
        }
        CombatEvent::SpellRefused {
            caster,
            spell,
            error,
        } if Some(*caster) == run.player => {
            log(run, format!("{spell} refused: {error:?}"));
        }
        CombatEvent::SpellInterrupted { caster, .. } => {
            log(run, format!("{}'s verse broke", name_of(run, *caster)))
        }
        CombatEvent::Overdrawn { debt, .. } => log(
            run,
            format!("Overdrawn: {debt:.1} lamps on the Wheel's credit"),
        ),
        CombatEvent::Alerted { by, .. } => {
            log(run, format!("{} raised the alarm", name_of(run, *by)))
        }
        CombatEvent::Fled { who } => log(run, format!("{} broke and ran", name_of(run, *who))),
        CombatEvent::Kindled { who, on } if Some(*who) == run.player => {
            log(
                run,
                format!("Kindled Body {}", if *on { "on" } else { "off" }),
            );
        }
        CombatEvent::Focus { active, .. } => log(
            run,
            format!("Focus {}", if *active { "on: time slows" } else { "off" }),
        ),
        CombatEvent::PartBroken { target, part } => {
            log(run, format!("{}'s {part} broke", name_of(run, *target)))
        }
        CombatEvent::ProjectileEnded {
            point,
            normal,
            target: None,
            element,
            ..
        } => {
            if *normal != Vec3::ZERO {
                burst(ctx.world, run, *point, *element, false, 12);
                if normal.y > 0.5 {
                    match element {
                        Some(Element::Flame) => mark(ctx.world, run, *point, 1.6, "scorch"),
                        Some(Element::Frost) => mark(ctx.world, run, *point, 1.6, "frostmark"),
                        _ => {}
                    }
                }
            }
        }
        CombatEvent::Bloomed {
            at,
            radius,
            element,
        } => {
            burst(
                ctx.world,
                run,
                *at + Vec3::Y * 0.5,
                Some(*element),
                false,
                48,
            );
            match element {
                Element::Flame => mark(ctx.world, run, *at, radius * 2.4, "scorch"),
                Element::Frost => mark(ctx.world, run, *at, radius * 2.4, "frostmark"),
                _ => {}
            }
        }
        CombatEvent::Died {
            target,
            killer,
            impulse,
            point,
        } => {
            let by = killer.map_or_else(String::new, |k| format!(" to {}", name_of(run, k)));
            log(run, format!("{} fell{by}", name_of(run, *target)));
            ragdoll(ctx, run, *target, *impulse, *point);
        }
        CombatEvent::Surface(change) => {
            let key = (change.x / 2, change.z / 2);
            if let (Some((entity, shown)), Some(field)) =
                (run.tiles.get_mut(&key), &run.world.environment)
            {
                let now = tile_surface(field, key.0, key.1);
                if now != *shown {
                    *shown = now;
                    if let Some(m) = ctx.world.get_mut::<MaterialComponent>(*entity) {
                        *m = material(surface_material(now));
                    }
                }
            }
        }
        _ => {}
    }
}

/// A fallen fighter becomes a two-part Jolt ragdoll pushed by the blow.
fn ragdoll(ctx: &mut EngineContext, run: &mut Run, id: CombatantId, impulse: Vec3, point: Vec3) {
    let Some(c) = run.world.combatant(id) else {
        return;
    };
    if c.mount_def.is_some() {
        // A fallen mount lies where it fell; its rider was thrown.
        return;
    }
    let feet = c.position;
    let rotation = yaw(c.facing);
    let torso = feet + Vec3::Y * BODY_HEIGHT * 0.5;
    let neck = feet + Vec3::Y * BODY_HEIGHT;
    let parts = [
        somnium_physics::animation::RagdollPart {
            parent: None,
            position: torso,
            rotation,
            half_height: BODY_HEIGHT * 0.5 - BODY_WIDTH * 0.5,
            radius: BODY_WIDTH * 0.5,
            anchor: torso,
            swing_limit: 0.0,
            twist_limit: 0.0,
        },
        somnium_physics::animation::RagdollPart {
            parent: Some(0),
            position: neck + Vec3::Y * HEAD * 0.5,
            rotation,
            half_height: 0.02,
            radius: HEAD * 0.5,
            anchor: neck,
            swing_limit: 0.6,
            twist_limit: 0.4,
        },
    ];
    let Ok(ragdoll) = ctx.physics.create_ragdoll(&parts) else {
        return;
    };
    if let Ok(bodies) = ctx.physics.ragdoll_bodies(ragdoll) {
        let body = bodies[0];
        // A little up and the blow's own push, at the torso.
        let push = impulse.clamp_length_max(400.0) + Vec3::Y * 60.0;
        ctx.physics.add_impulse(body, push);
        let _ = point;
    }
    run.ragdolls.insert(id, ragdoll);
}

fn write_look(world: &mut World, looks: &mut HashMap<Entity, Look>, entity: Entity, look: Look) {
    let changed = looks.get(&entity).is_none_or(|old| match (old, look) {
        (Look::Hidden, Look::Hidden) => false,
        (Look::Shown(a, b, c), Look::Shown(x, y, z)) => {
            a.distance_squared(x) > 1e-6 || b.dot(y).abs() < 0.99999 || c.distance_squared(z) > 1e-6
        }
        _ => true,
    });
    if !changed {
        return;
    }
    looks.insert(entity, look);
    if let Some(t) = world.get_mut::<Transform>(entity) {
        match look {
            Look::Hidden => t.translation = PARKED,
            Look::Shown(p, r, s) => {
                t.translation = p;
                t.rotation = r;
                t.scale = s;
            }
        }
    }
}

fn set_material(world: &mut World, entity: Entity, slot: &mut &'static str, want: &'static str) {
    if *slot != want {
        *slot = want;
        if let Some(m) = world.get_mut::<MaterialComponent>(entity) {
            *m = material(want);
        }
    }
}

/// Fighters, ragdolls, missiles, walls, blooms, waves, fire and readouts.
fn write_scene(ctx: &mut EngineContext, run: &mut Run) {
    let world = &mut *ctx.world;
    // Fighters: world pose into the body's parent space.
    for (id, entity) in &run.bodies {
        let Some(c) = run.world.combatant(*id) else {
            continue;
        };
        let size =
            run.dims
                .get(id)
                .copied()
                .unwrap_or(Vec3::new(BODY_WIDTH, BODY_HEIGHT, BODY_WIDTH));
        let pose = match run
            .ragdolls
            .get(id)
            .and_then(|r| ctx.physics.ragdoll_pose(*r).ok())
        {
            Some(parts) => {
                let (_, r, p) = parts[0].to_scale_rotation_translation();
                (p, r)
            }
            None => (c.position + Vec3::Y * size.y * 0.5, yaw(c.facing)),
        };
        let parent = world
            .get::<Parent>(*entity)
            .map(|p| p.entity)
            .filter(|p| *p != Entity::DANGLING)
            .map_or(Mat4::IDENTITY, |p| world_matrix(world, p));
        let local = parent.inverse() * Mat4::from_scale_rotation_translation(size, pose.1, pose.0);
        let (s, r, p) = local.to_scale_rotation_translation();
        if let Some(t) = world.get_mut::<Transform>(*entity)
            && (t.translation.distance_squared(p) > 1e-6 || t.rotation.dot(r).abs() < 0.99999)
        {
            t.translation = p;
            t.rotation = r;
            t.scale = s;
        }
        // Live readouts, rounded so regeneration does not write every step.
        let reading = (
            (c.vitals.health.current * 10.0).round() as i32,
            (c.vitals.stamina.current).round() as i32,
            (c.vitals.mana.current * 10.0).round() as i32,
            c.action.label(),
        );
        if run.readouts.get(entity) != Some(&reading) {
            run.readouts.insert(*entity, reading);
            if let Some(cc) = world.get_mut::<CombatantComponent>(*entity) {
                cc.health = c.vitals.health.current;
                cc.stamina = c.vitals.stamina.current;
                cc.mana = c.vitals.mana.current;
                cc.state = c.action.label().to_string();
            }
        }
    }
    let pools = &mut run.pools;
    // Missiles.
    let flying = run.world.projectiles();
    for (i, (entity, slot)) in pools.projectiles.iter_mut().enumerate() {
        let look = flying.get(i).map(|p| {
            let (scale, mat) = match (p.def.kind, p.spell.as_ref().map(|s| s.element)) {
                (ProjectileKind::Spell, Some(e)) => (
                    Vec3::splat((p.def.radius * 2.0).max(0.2)),
                    element_material(e),
                ),
                (ProjectileKind::WindCut, _) => (Vec3::new(0.9, 0.06, 0.25), "gale"),
                (ProjectileKind::Round, _) => (Vec3::splat(0.06), "dark"),
                _ => (
                    Vec3::new(0.05, 0.05, 0.7),
                    if p.burning { "fire" } else { "dark" },
                ),
            };
            (Look::Shown(p.position, yaw(p.velocity), scale), mat)
        });
        if let Some((_, mat)) = look {
            set_material(world, *entity, slot, mat);
        }
        write_look(
            world,
            &mut pools.looks,
            *entity,
            look.map_or(Look::Hidden, |l| l.0),
        );
    }
    // Walls.
    let walls = run.world.walls();
    for (i, (entity, slot)) in pools.walls.iter_mut().enumerate() {
        let look = walls.get(i).map(|w| {
            (
                Look::Shown(w.obb.center, w.obb.rotation, w.obb.half * 2.0),
                element_material(w.element),
            )
        });
        if let Some((_, mat)) = look {
            set_material(world, *entity, slot, mat);
        }
        write_look(
            world,
            &mut pools.looks,
            *entity,
            look.map_or(Look::Hidden, |l| l.0),
        );
    }
    // Blooms gathering: a ring of beads on the ground where it will burst.
    let blooms = run.world.blooms();
    for (i, (entity, slot)) in pools.blooms.iter_mut().enumerate() {
        let (ring, bead) = (i / RING_BEADS, i % RING_BEADS);
        let look = blooms.get(ring).map(|b| {
            let a = bead as f32 / RING_BEADS as f32 * std::f32::consts::TAU;
            let at = Vec3::new(
                b.center.x + a.cos() * b.radius,
                run.floor + 0.12,
                b.center.z + a.sin() * b.radius,
            );
            (
                Look::Shown(at, Quat::IDENTITY, Vec3::splat(0.22)),
                element_material(b.element),
            )
        });
        if let Some((_, mat)) = look {
            set_material(world, *entity, slot, mat);
        }
        write_look(
            world,
            &mut pools.looks,
            *entity,
            look.map_or(Look::Hidden, |l| l.0),
        );
    }
    // Waves: beads along each front.
    let waves = run.world.waves();
    for (i, entity) in pools.beads.iter().enumerate() {
        let (w, k) = (i / WAVE_BEADS, i % WAVE_BEADS);
        let look = waves.get(w).map_or(Look::Hidden, |wave| {
            let a = -wave.half_angle + 2.0 * wave.half_angle * k as f32 / (WAVE_BEADS - 1) as f32;
            let p =
                wave.origin + Quat::from_rotation_y(-a) * wave.dir * wave.radius + Vec3::Y * 0.9;
            Look::Shown(p, Quat::IDENTITY, Vec3::splat(0.35))
        });
        write_look(world, &mut pools.looks, *entity, look);
    }
    // Fire on the ground.
    let burning: Vec<Vec2> = run
        .world
        .environment
        .as_ref()
        .map(|f| {
            let all: Vec<Vec2> = f.burning().collect();
            let stride = (all.len() / FIRES).max(1);
            all.into_iter().step_by(stride).take(FIRES).collect()
        })
        .unwrap_or_default();
    for (i, (entity, lit)) in pools.fires.iter_mut().enumerate() {
        let at = burning
            .get(i)
            .map(|c| Vec3::new(c.x, run.floor + 0.05, c.y));
        let want = at.is_some();
        if let Some(at) = at {
            write_look(
                world,
                &mut pools.looks,
                *entity,
                Look::Shown(at, Quat::IDENTITY, Vec3::ONE),
            );
        }
        if *lit != want {
            *lit = want;
            if let Some(p) = world.get_mut::<ParticleEmitter>(*entity) {
                p.enabled = want;
            }
        }
    }
}

fn player_lines(run: &Run) -> (String, String) {
    let Some(c) = run.player.and_then(|p| run.world.combatant(p)) else {
        return ("No player-controlled fighter".into(), String::new());
    };
    let v = &c.vitals;
    let mut flags = Vec::new();
    if c.kindled {
        flags.push("Kindled".to_string());
    }
    if let Some(t) = c.lock.and_then(|t| run.world.combatant(t)) {
        flags.push(format!("Lock: {}", t.name));
    }
    if v.overdrawn() {
        flags.push("Overdrawn".into());
    }
    if let Some(w) = c.ward {
        flags.push(format!("Ward {:.0}", w.strength));
    }
    if run.world.focusing() == Some(c.id) {
        flags.push("Focus".into());
    }
    for kind in StatusKind::ALL {
        if c.status.active(kind) {
            flags.push(match kind {
                StatusKind::Chill | StatusKind::Bleed => {
                    format!("{} {:.0}", kind.name(), c.status.buildup(kind))
                }
                _ => kind.name().to_string(),
            });
        }
    }
    let state = if matches!(c.action, Action::Dead) {
        "Fallen"
    } else {
        c.action.label()
    };
    let vitals = format!(
        "{} - {state}\nHealth {:.0}/{:.0}   Stamina {:.0}/{:.0}   Poise {:.0}\nMana {:.1}/{:.0} lamps   Focus {:.0}\n{}",
        c.name,
        v.health.current,
        v.health.max,
        v.stamina.current,
        v.stamina.max,
        v.poise.current,
        v.mana.current,
        v.mana.max,
        v.focus.current,
        flags.join(" | ")
    );
    let now = run.world.time();
    let slots: Vec<String> = c
        .magic
        .hotbar
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| {
            let slot = slot.as_ref()?;
            let name = run
                .world
                .spell(&slot.spell)
                .map_or(slot.spell.clone(), |s| s.name.clone());
            let key = if i == 9 { 0 } else { i + 1 };
            let cooling = c
                .cooldowns
                .get(&slot.spell)
                .map_or(0.0, |r| (r - now).max(0.0));
            Some(if cooling > 0.0 {
                format!("{key} {name} ({cooling:.1}s)")
            } else {
                format!("{key} {name}")
            })
        })
        .collect();
    let shaping = c.shaping.as_ref().map_or_else(
        || "Hold a number to shape: the mouse aims, the wheel edits the measure, R picks which".to_string(),
        |s| {
            let m = s.request.measure;
            let size = rote_name(m.size, &ROTE_SIZE).map_or_else(|| format!("{:.2} m", m.size), |r| format!("{r} ({:.2} m)", m.size));
            let axis = match s.axis {
                MeasureAxis::Size => "size",
                MeasureAxis::Range => "range",
                MeasureAxis::Speed => "speed",
                MeasureAxis::Count => "count",
                MeasureAxis::Duration => "duration",
            };
            let cost = match &s.preview {
                Ok(cost) => format!("{:.1} lamps, {:.1} s", cost.mana, cost.gather),
                Err(e) => format!("refused: {e:?}"),
            };
            format!(
                "{} {:?}: size {size}, range {:.0} m, speed {:.0} m/s, x{} | wheel: {axis} | {cost}",
                run.world.spell(&s.request.spell).map_or(s.request.spell.as_str(), |a| a.name.as_str()),
                s.request.shape,
                m.range,
                m.speed,
                m.count,
            )
        },
    );
    (vitals, format!("{}\n{shaping}", slots.join("   ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every material the sandbox names exists, and the round marks point at
    /// the smoke sprite by the engine's own asset id.
    #[test]
    fn the_sandbox_materials_load_and_the_marks_are_round() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/materials/combat");
        for name in [
            "floor",
            "party",
            "raider",
            "skin",
            "dark",
            "water",
            "ice",
            "grass",
            "snow",
            "ash",
            "stone",
            "fire",
            "gale",
            "force",
            "mending",
            "frost",
            "hide",
            "scorch",
            "frostmark",
        ] {
            let asset = somnium_asset::material::load_material(dir.join(format!("{name}.sommat")))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            if matches!(name, "scorch" | "frostmark") {
                assert_eq!(
                    asset.albedo_map,
                    AssetId::from_relative_path(std::path::Path::new(
                        "textures/particles/smoke_01.png"
                    )),
                    "{name} uses the soft round sprite"
                );
            }
        }
    }

    /// The preset builds a fight the combat world accepts, and the fight runs
    /// headless through the same `build` Play uses.
    #[test]
    fn the_sandbox_builds_and_fights_without_a_window() {
        let mut world = World::new();
        let spawned = spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        assert!(spawned.len() > 20);
        assert!(claims_play(&world));
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        assert_eq!(run.world.combatants().count(), 10);
        let elk = run
            .world
            .combatants()
            .find(|c| c.name == "Frost-elk")
            .expect("the elk");
        assert!(elk.mount_def.is_some() && elk.brain.is_none());
        assert!(run.player.is_some());
        assert!(run.camera.is_some());
        let field = run.world.environment.as_ref().expect("ground");
        let census = field.census();
        assert!(census[Surface::Water as usize] > 50 && census[Surface::Grass as usize] > 100);
        let feet = run.world.combatant(run.player.unwrap()).unwrap().position;
        assert!(feet.y.abs() < 0.1, "feet on the floor: {feet}");
        let mut events = Vec::new();
        for _ in 0..(20.0 / (1.0 / 60.0)) as usize {
            run.world.step(1.0 / 60.0, &somnium_combat::OpenField, None);
            events.extend(run.world.drain_events());
        }
        let harmed = events
            .iter()
            .filter(|e| matches!(e, CombatEvent::Damaged { .. }))
            .count();
        let cast = events
            .iter()
            .filter(|e| matches!(e, CombatEvent::SpellReleased { .. }))
            .count();
        assert!(harmed > 10, "they fought: {harmed} hits");
        assert!(cast > 0, "the mage cast");
        let (vitals, hotbar) = player_lines(&run);
        assert!(vitals.contains("Health") && hotbar.contains("Ice Lance"));
    }
}
