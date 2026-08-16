//! Nocturne Atelier icon atlas.
//!
//! The original Phase 26 glyphs remain at stable indices. Phase 26-Zeta appends
//! the approved Somnium-specific extension, drawn on the supplied 24×24 / 2 px
//! grid. The engine mark implements the approved original Eclipse-S route.

use glam::Vec2;

use crate::types::Rect;

/// wgpu texture_id reserved for the icon atlas.
pub const ICON_ATLAS_TEXTURE_ID: u32 = 1;

/// Small cut. Serves the 16 / 20 / 24 px chrome sizes; 32 → 16 is an exact
/// 2:1 box filter under linear sampling.
pub const ICON_CELL: u32 = 32;
/// Large cut. Content Browser tiles draw at [`crate::theme::ICON_DRAWER`] =
/// 80 px, and upscaling the 32 px cell by 2.5× is what made them look
/// pixelated. 96 → 80 is a mild downscale instead.
pub const ICON_CELL_LARGE: u32 = 96;
pub const ICON_ATLAS_WIDTH: u32 = 1024;
pub const ICON_ATLAS_HEIGHT: u32 = 1024;
const CELLS_PER_ROW: u32 = ICON_ATLAS_WIDTH / ICON_CELL;
const LARGE_PER_ROW: u32 = ICON_ATLAS_WIDTH / ICON_CELL_LARGE;
/// The large block starts below the small one. Three rows of 32 px cover all
/// 84 glyphs, so the large block begins at 96.
const LARGE_ORIGIN_Y: u32 = 96;
/// Above this drawn size a glyph samples the large cut. 40 sits between the
/// 24 px action icons and the 80 px drawer tiles.
const LARGE_CUT_THRESHOLD: f32 = 40.0;

/// Named glyphs packed into the atlas. Order is the cell index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum IconId {
    EngineMark = 0,
    File,
    Edit,
    View,
    Window,
    Help,
    HelpCircle,
    Save,
    Undo,
    Redo,
    Play,
    Pause,
    Stop,
    Translate,
    Rotate,
    Scale,
    Select,
    Landscape,
    Foliage,
    Search,
    Filter,
    Settings,
    Dock,
    Close,
    Folder,
    FolderOpen,
    Chevron,
    ChevronDown,
    Visibility,
    Add,
    Delete,
    Duplicate,
    Import,
    Profiler,
    OutputLog,
    ContentDrawer,
    Cube,
    Sphere,
    Plane,
    Cylinder,
    DirectionalLight,
    PointLight,
    SpotLight,
    Particle,
    Terrain,
    VoxelTerrain,
    EmptyEntity,
    Camera,
    Mesh,
    Texture,
    Material,
    Scene,
    Audio,
    Shader,
    Font,
    Script,
    Json,
    License,
    Unknown,
    Derived,
    Transform,
    Light,
    PostFx,
    Water,
    Vessel,
    Ok,
    Warn,
    Error,
    Check,
    Minimize,
    Maximize,
    ImmersivePlay,
    SculptRaise,
    SculptLower,
    SculptSmooth,
    SculptFlatten,
    SculptNoise,
    PaintLayer,
    FoliagePaint,
    FoliageErase,
    FoliageSingle,
    LightProbe,
    MaterialGraph,
    RayTrace,
}

impl IconId {
    pub const ALL: &'static [IconId] = &[
        Self::EngineMark,
        Self::File,
        Self::Edit,
        Self::View,
        Self::Window,
        Self::Help,
        Self::HelpCircle,
        Self::Save,
        Self::Undo,
        Self::Redo,
        Self::Play,
        Self::Pause,
        Self::Stop,
        Self::Translate,
        Self::Rotate,
        Self::Scale,
        Self::Select,
        Self::Landscape,
        Self::Foliage,
        Self::Search,
        Self::Filter,
        Self::Settings,
        Self::Dock,
        Self::Close,
        Self::Folder,
        Self::FolderOpen,
        Self::Chevron,
        Self::ChevronDown,
        Self::Visibility,
        Self::Add,
        Self::Delete,
        Self::Duplicate,
        Self::Import,
        Self::Profiler,
        Self::OutputLog,
        Self::ContentDrawer,
        Self::Cube,
        Self::Sphere,
        Self::Plane,
        Self::Cylinder,
        Self::DirectionalLight,
        Self::PointLight,
        Self::SpotLight,
        Self::Particle,
        Self::Terrain,
        Self::VoxelTerrain,
        Self::EmptyEntity,
        Self::Camera,
        Self::Mesh,
        Self::Texture,
        Self::Material,
        Self::Scene,
        Self::Audio,
        Self::Shader,
        Self::Font,
        Self::Script,
        Self::Json,
        Self::License,
        Self::Unknown,
        Self::Derived,
        Self::Transform,
        Self::Light,
        Self::PostFx,
        Self::Water,
        Self::Vessel,
        Self::Ok,
        Self::Warn,
        Self::Error,
        Self::Check,
        Self::Minimize,
        Self::Maximize,
        Self::ImmersivePlay,
        Self::SculptRaise,
        Self::SculptLower,
        Self::SculptSmooth,
        Self::SculptFlatten,
        Self::SculptNoise,
        Self::PaintLayer,
        Self::FoliagePaint,
        Self::FoliageErase,
        Self::FoliageSingle,
        Self::LightProbe,
        Self::MaterialGraph,
        Self::RayTrace,
    ];

    pub fn index(self) -> u32 {
        self as u32
    }

    /// Top-left pixel of this glyph's small cell.
    pub fn cell_px(self) -> (u32, u32) {
        let i = self.index();
        (
            (i % CELLS_PER_ROW) * ICON_CELL,
            (i / CELLS_PER_ROW) * ICON_CELL,
        )
    }

    /// Top-left pixel of this glyph's large cell.
    pub fn cell_px_large(self) -> (u32, u32) {
        let i = self.index();
        (
            (i % LARGE_PER_ROW) * ICON_CELL_LARGE,
            LARGE_ORIGIN_Y + (i / LARGE_PER_ROW) * ICON_CELL_LARGE,
        )
    }

    fn uv_for(origin: (u32, u32), cell: u32) -> (Vec2, Vec2) {
        let u0 = origin.0 as f32 / ICON_ATLAS_WIDTH as f32;
        let v0 = origin.1 as f32 / ICON_ATLAS_HEIGHT as f32;
        let du = cell as f32 / ICON_ATLAS_WIDTH as f32;
        let dv = cell as f32 / ICON_ATLAS_HEIGHT as f32;
        (Vec2::new(u0, v0), Vec2::new(u0 + du, v0 + dv))
    }

    pub fn uv_rect(self) -> (Vec2, Vec2) {
        Self::uv_for(self.cell_px(), ICON_CELL)
    }

    pub fn uv_rect_large(self) -> (Vec2, Vec2) {
        Self::uv_for(self.cell_px_large(), ICON_CELL_LARGE)
    }

    /// Pick the cut that suits the size the glyph is being drawn at.
    ///
    /// Callers do not choose: they pass the destination rect they already have,
    /// so a widget that grows past the chrome sizes picks up the high-resolution
    /// cut without anyone remembering to ask for it.
    pub fn draw_quad(self, dest: Rect) -> ([Vec2; 4], u32) {
        let (uv0, uv1) = if dest.w.max(dest.h) > LARGE_CUT_THRESHOLD {
            self.uv_rect_large()
        } else {
            self.uv_rect()
        };
        (
            [
                Vec2::new(uv0.x, uv0.y),
                Vec2::new(uv1.x, uv0.y),
                Vec2::new(uv1.x, uv1.y),
                Vec2::new(uv0.x, uv1.y),
            ],
            ICON_ATLAS_TEXTURE_ID,
        )
    }
}

pub struct IconAtlas {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub dirty: bool,
}

impl IconAtlas {
    pub fn new() -> Self {
        let mut atlas = Self {
            pixels: vec![0u8; (ICON_ATLAS_WIDTH * ICON_ATLAS_HEIGHT * 4) as usize],
            width: ICON_ATLAS_WIDTH,
            height: ICON_ATLAS_HEIGHT,
            dirty: true,
        };
        for &id in IconId::ALL {
            // Zeta-E: the vendored SVG family is authoritative. The procedural
            // strokes below stay as the fallback for any glyph without a
            // source, so a new `IconId` variant degrades to hand-drawn art
            // rather than to an empty cell.
            //
            // Each glyph is rasterized twice, once per cut. Re-rendering the
            // vector at 96 px is what makes a Content Browser tile crisp;
            // upscaling the 32 px cell cannot recover the strokes.
            match crate::icon_svg::source_for(id) {
                Some(svg) => {
                    if let Some(mask) = crate::icon_svg::rasterize(svg, ICON_CELL) {
                        let (ox, oy) = id.cell_px();
                        atlas.blit_mask(ox as i32, oy as i32, ICON_CELL as usize, &mask);
                    } else {
                        atlas.rasterize(id);
                    }
                    if let Some(mask) = crate::icon_svg::rasterize(svg, ICON_CELL_LARGE) {
                        let (ox, oy) = id.cell_px_large();
                        atlas.blit_mask(ox as i32, oy as i32, ICON_CELL_LARGE as usize, &mask);
                    }
                }
                None => atlas.rasterize(id),
            }
        }
        atlas
    }

    /// Copy a rasterized alpha mask into the atlas. RGB stays white so the UI
    /// shader tints the glyph with the widget's semantic colour.
    fn blit_mask(&mut self, ox: i32, oy: i32, cell: usize, mask: &[u8]) {
        for y in 0..cell {
            for x in 0..cell {
                let a = mask.get(y * cell + x).copied().unwrap_or(0);
                if a > 0 {
                    self.put(ox + x as i32, oy + y as i32, a);
                }
            }
        }
    }

    fn cell_origin(id: IconId) -> (i32, i32) {
        let i = id.index();
        let col = (i % CELLS_PER_ROW) as i32;
        let row = (i / CELLS_PER_ROW) as i32;
        (col * ICON_CELL as i32, row * ICON_CELL as i32)
    }

    fn put(&mut self, x: i32, y: i32, a: u8) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        if x >= self.width || y >= self.height {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        // Max-blend coverage into white RGB so vertex colour tints the icon.
        self.pixels[i] = 255;
        self.pixels[i + 1] = 255;
        self.pixels[i + 2] = 255;
        self.pixels[i + 3] = self.pixels[i + 3].max(a);
    }

    fn stamp(&mut self, x: f32, y: f32, coverage: f32) {
        let a = (coverage.clamp(0.0, 1.0) * 255.0) as u8;
        if a == 0 {
            return;
        }
        self.put(x.floor() as i32, y.floor() as i32, a);
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let steps = (len * 2.0).ceil() as i32;
        let nx = -dy / len;
        let ny = dx / len;
        let half = width * 0.5;
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let x = x0 + dx * t;
            let y = y0 + dy * t;
            for k in -2..=2 {
                let o = k as f32 * 0.5;
                let d = (o.abs() - half).max(0.0);
                let cov = (1.0 - d).clamp(0.0, 1.0);
                self.stamp(x + nx * o, y + ny * o, cov);
            }
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, width: f32, fill: bool) {
        let outer = r + width * 0.5;
        let inner = if fill {
            0.0
        } else {
            (r - width * 0.5).max(0.0)
        };
        let minx = (cx - outer - 1.0).floor() as i32;
        let maxx = (cx + outer + 1.0).ceil() as i32;
        let miny = (cy - outer - 1.0).floor() as i32;
        let maxy = (cy + outer + 1.0).ceil() as i32;
        for y in miny..=maxy {
            for x in minx..=maxx {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = if fill {
                    (outer + 0.5 - d).clamp(0.0, 1.0)
                } else {
                    let a = (outer + 0.5 - d).clamp(0.0, 1.0);
                    let b = (d - inner + 0.5).clamp(0.0, 1.0);
                    a.min(b)
                };
                self.stamp(x as f32, y as f32, cov);
            }
        }
    }

    fn rect_stroke(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32) {
        self.line(x, y, x + w, y, width);
        self.line(x + w, y, x + w, y + h, width);
        self.line(x + w, y + h, x, y + h, width);
        self.line(x, y + h, x, y, width);
    }

    fn arc(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        start_degrees: f32,
        end_degrees: f32,
        width: f32,
    ) {
        let span = end_degrees - start_degrees;
        let steps = ((span.abs().to_radians() * radius * 2.0).ceil() as usize).max(8);
        let point = |degrees: f32| {
            let angle = degrees.to_radians();
            (cx + angle.cos() * radius, cy + angle.sin() * radius)
        };
        let mut previous = point(start_degrees);
        for step in 1..=steps {
            let degrees = start_degrees + span * step as f32 / steps as f32;
            let current = point(degrees);
            self.line(previous.0, previous.1, current.0, current.1, width);
            previous = current;
        }
    }

    fn polyline(&mut self, points: &[(f32, f32)], width: f32) {
        for pair in points.windows(2) {
            self.line(pair[0].0, pair[0].1, pair[1].0, pair[1].1, width);
        }
    }

    fn rasterize(&mut self, id: IconId) {
        let (ox, oy) = Self::cell_origin(id);
        let ox = ox as f32;
        let oy = oy as f32;
        // 24px drawing box centered in 32px cell.
        let s = |v: f32| ox + 4.0 + v;
        let t = |v: f32| oy + 4.0 + v;
        let w = 1.8;
        match id {
            IconId::EngineMark => {
                // Eclipse S: two counter-rotating crescent blades separated by
                // a diagonal channel. This is an optical 24 px interpretation
                // of assets/brand/somnium-s-eclipse.svg, not a font glyph.
                self.arc(s(9.0), t(8.0), 6.1, -42.0, -292.0, 3.4);
                self.arc(s(15.0), t(16.0), 6.1, 138.0, 388.0, 3.4);
            }
            IconId::File => {
                self.line(s(8.0), t(4.0), s(14.0), t(4.0), w);
                self.line(s(14.0), t(4.0), s(17.0), t(7.0), w);
                self.line(s(17.0), t(7.0), s(17.0), t(20.0), w);
                self.line(s(17.0), t(20.0), s(8.0), t(20.0), w);
                self.line(s(8.0), t(20.0), s(8.0), t(4.0), w);
                self.line(s(14.0), t(4.0), s(14.0), t(7.0), w);
                self.line(s(14.0), t(7.0), s(17.0), t(7.0), w);
            }
            IconId::Edit => {
                self.line(s(4.0), t(20.0), s(8.0), t(19.0), w);
                self.line(s(8.0), t(19.0), s(19.0), t(8.0), w);
                self.line(s(19.0), t(8.0), s(16.0), t(5.0), w);
                self.line(s(16.0), t(5.0), s(5.0), t(16.0), w);
                self.line(s(5.0), t(16.0), s(4.0), t(20.0), w);
            }
            IconId::View => {
                self.circle(s(12.0), t(12.0), 3.0, w, false);
                self.line(s(3.0), t(12.0), s(7.0), t(8.0), w);
                self.line(s(7.0), t(8.0), s(17.0), t(8.0), w);
                self.line(s(17.0), t(8.0), s(21.0), t(12.0), w);
                self.line(s(21.0), t(12.0), s(17.0), t(16.0), w);
                self.line(s(17.0), t(16.0), s(7.0), t(16.0), w);
                self.line(s(7.0), t(16.0), s(3.0), t(12.0), w);
            }
            IconId::Window => self.rect_stroke(s(4.0), t(5.0), 16.0, 14.0, w),
            IconId::Help | IconId::HelpCircle => {
                self.circle(s(12.0), t(12.0), 9.0, w, false);
                self.circle(s(12.0), t(17.5), 1.1, w, true);
                self.line(s(12.0), t(14.0), s(12.0), t(12.5), w);
                self.line(s(12.0), t(12.5), s(14.5), t(10.0), w);
                self.line(s(14.5), t(10.0), s(12.0), t(7.5), w);
                self.line(s(12.0), t(7.5), s(9.5), t(9.0), w);
            }
            IconId::Save => {
                self.rect_stroke(s(5.0), t(5.0), 14.0, 14.0, w);
                self.rect_stroke(s(8.0), t(5.0), 8.0, 6.0, w);
            }
            IconId::Undo => {
                self.line(s(7.0), t(8.0), s(4.0), t(12.0), w);
                self.line(s(4.0), t(12.0), s(7.0), t(16.0), w);
                self.line(s(4.0), t(12.0), s(16.0), t(12.0), w);
                self.circle(s(16.0), t(14.0), 4.0, w, false);
            }
            IconId::Redo => {
                self.line(s(17.0), t(8.0), s(20.0), t(12.0), w);
                self.line(s(20.0), t(12.0), s(17.0), t(16.0), w);
                self.line(s(20.0), t(12.0), s(8.0), t(12.0), w);
                self.circle(s(8.0), t(14.0), 4.0, w, false);
            }
            IconId::Play => {
                self.line(s(8.0), t(6.0), s(8.0), t(18.0), w);
                self.line(s(8.0), t(6.0), s(18.0), t(12.0), w);
                self.line(s(18.0), t(12.0), s(8.0), t(18.0), w);
            }
            IconId::Pause => {
                self.line(s(8.0), t(6.0), s(8.0), t(18.0), w + 1.2);
                self.line(s(16.0), t(6.0), s(16.0), t(18.0), w + 1.2);
            }
            IconId::Stop => self.rect_stroke(s(7.0), t(7.0), 10.0, 10.0, w + 0.4),
            IconId::Translate => {
                self.line(s(12.0), t(4.0), s(12.0), t(20.0), w);
                self.line(s(4.0), t(12.0), s(20.0), t(12.0), w);
                self.line(s(12.0), t(4.0), s(9.0), t(7.0), w);
                self.line(s(12.0), t(4.0), s(15.0), t(7.0), w);
            }
            IconId::Rotate => {
                self.circle(s(12.0), t(13.0), 7.0, w, false);
                self.line(s(18.0), t(7.0), s(20.0), t(11.0), w);
                self.line(s(20.0), t(11.0), s(16.0), t(10.0), w);
            }
            IconId::Scale => {
                self.rect_stroke(s(5.0), t(9.0), 8.0, 8.0, w);
                self.rect_stroke(s(11.0), t(5.0), 8.0, 8.0, w);
            }
            IconId::Select => {
                self.line(s(8.0), t(4.0), s(8.0), t(16.0), w);
                self.line(s(8.0), t(16.0), s(11.0), t(13.0), w);
                self.line(s(11.0), t(13.0), s(14.0), t(20.0), w);
            }
            IconId::Landscape => {
                self.line(s(3.0), t(17.0), s(9.0), t(9.0), w);
                self.line(s(9.0), t(9.0), s(13.0), t(14.0), w);
                self.line(s(13.0), t(14.0), s(21.0), t(6.0), w);
                self.line(s(3.0), t(19.0), s(21.0), t(19.0), w);
            }
            IconId::Foliage => {
                self.line(s(12.0), t(20.0), s(12.0), t(12.0), w);
                self.circle(s(12.0), t(9.0), 6.0, w, false);
                self.circle(s(8.0), t(12.0), 3.5, w, false);
            }
            IconId::Search => {
                self.circle(s(10.0), t(10.0), 6.0, w, false);
                self.line(s(15.0), t(15.0), s(20.0), t(20.0), w);
            }
            IconId::Filter => {
                self.line(s(4.0), t(6.0), s(20.0), t(6.0), w);
                self.line(s(7.0), t(12.0), s(17.0), t(12.0), w);
                self.line(s(10.0), t(18.0), s(14.0), t(18.0), w);
            }
            IconId::Settings => {
                self.circle(s(12.0), t(12.0), 4.0, w, false);
                self.circle(s(12.0), t(12.0), 9.0, w, false);
            }
            IconId::Dock => {
                self.rect_stroke(s(4.0), t(5.0), 16.0, 14.0, w);
                self.line(s(4.0), t(14.0), s(20.0), t(14.0), w);
            }
            IconId::Close => {
                self.line(s(6.0), t(6.0), s(18.0), t(18.0), w);
                self.line(s(18.0), t(6.0), s(6.0), t(18.0), w);
            }
            IconId::Folder | IconId::FolderOpen => {
                self.line(s(4.0), t(8.0), s(9.0), t(8.0), w);
                self.line(s(9.0), t(8.0), s(11.0), t(10.0), w);
                self.line(s(11.0), t(10.0), s(20.0), t(10.0), w);
                self.line(s(20.0), t(10.0), s(20.0), t(18.0), w);
                self.line(s(20.0), t(18.0), s(4.0), t(18.0), w);
                self.line(s(4.0), t(18.0), s(4.0), t(8.0), w);
            }
            IconId::Chevron => {
                self.line(s(9.0), t(6.0), s(15.0), t(12.0), w);
                self.line(s(15.0), t(12.0), s(9.0), t(18.0), w);
            }
            IconId::ChevronDown => {
                self.line(s(6.0), t(9.0), s(12.0), t(15.0), w);
                self.line(s(12.0), t(15.0), s(18.0), t(9.0), w);
            }
            IconId::Visibility => {
                self.circle(s(12.0), t(12.0), 3.0, w, false);
                self.line(s(3.0), t(12.0), s(8.0), t(8.0), w);
                self.line(s(16.0), t(8.0), s(21.0), t(12.0), w);
                self.line(s(3.0), t(12.0), s(8.0), t(16.0), w);
                self.line(s(16.0), t(16.0), s(21.0), t(12.0), w);
            }
            IconId::Add => {
                self.line(s(12.0), t(5.0), s(12.0), t(19.0), w);
                self.line(s(5.0), t(12.0), s(19.0), t(12.0), w);
            }
            IconId::Delete => {
                self.rect_stroke(s(7.0), t(8.0), 10.0, 12.0, w);
                self.line(s(5.0), t(8.0), s(19.0), t(8.0), w);
                self.line(s(9.0), t(8.0), s(10.0), t(5.0), w);
                self.line(s(10.0), t(5.0), s(14.0), t(5.0), w);
            }
            IconId::Duplicate => {
                self.rect_stroke(s(7.0), t(7.0), 10.0, 10.0, w);
                self.rect_stroke(s(11.0), t(11.0), 10.0, 10.0, w);
            }
            IconId::Import => {
                self.line(s(12.0), t(4.0), s(12.0), t(14.0), w);
                self.line(s(12.0), t(14.0), s(8.0), t(10.0), w);
                self.line(s(12.0), t(14.0), s(16.0), t(10.0), w);
                self.line(s(5.0), t(18.0), s(19.0), t(18.0), w);
            }
            IconId::Profiler => {
                self.line(s(5.0), t(18.0), s(5.0), t(10.0), w);
                self.line(s(10.0), t(18.0), s(10.0), t(6.0), w);
                self.line(s(15.0), t(18.0), s(15.0), t(12.0), w);
                self.line(s(20.0), t(18.0), s(20.0), t(8.0), w);
            }
            IconId::OutputLog => {
                self.rect_stroke(s(4.0), t(5.0), 16.0, 14.0, w);
                self.line(s(7.0), t(9.0), s(17.0), t(9.0), w);
                self.line(s(7.0), t(13.0), s(14.0), t(13.0), w);
            }
            IconId::ContentDrawer => {
                self.rect_stroke(s(4.0), t(6.0), 16.0, 12.0, w);
                self.line(s(4.0), t(11.0), s(20.0), t(11.0), w);
                self.line(s(10.0), t(6.0), s(10.0), t(18.0), w);
            }
            IconId::Cube => self.rect_stroke(s(6.0), t(6.0), 12.0, 12.0, w),
            IconId::Sphere => self.circle(s(12.0), t(12.0), 8.0, w, false),
            IconId::Plane => {
                self.line(s(4.0), t(16.0), s(20.0), t(16.0), w);
                self.line(s(4.0), t(16.0), s(8.0), t(10.0), w);
                self.line(s(20.0), t(16.0), s(16.0), t(10.0), w);
            }
            IconId::Cylinder => {
                self.circle(s(12.0), t(7.0), 5.0, w, false);
                self.line(s(7.0), t(7.0), s(7.0), t(17.0), w);
                self.line(s(17.0), t(7.0), s(17.0), t(17.0), w);
                self.circle(s(12.0), t(17.0), 5.0, w, false);
            }
            IconId::DirectionalLight => {
                self.circle(s(12.0), t(12.0), 4.0, w, true);
                self.line(s(12.0), t(3.0), s(12.0), t(7.0), w);
                self.line(s(12.0), t(17.0), s(12.0), t(21.0), w);
                self.line(s(3.0), t(12.0), s(7.0), t(12.0), w);
                self.line(s(17.0), t(12.0), s(21.0), t(12.0), w);
            }
            IconId::PointLight => {
                self.circle(s(12.0), t(12.0), 3.5, w, true);
                self.circle(s(12.0), t(12.0), 8.0, w, false);
            }
            IconId::SpotLight => {
                self.line(s(12.0), t(5.0), s(6.0), t(19.0), w);
                self.line(s(12.0), t(5.0), s(18.0), t(19.0), w);
                self.line(s(6.0), t(19.0), s(18.0), t(19.0), w);
            }
            IconId::Particle => {
                self.circle(s(8.0), t(9.0), 2.0, w, true);
                self.circle(s(15.0), t(8.0), 1.5, w, true);
                self.circle(s(12.0), t(15.0), 2.5, w, true);
                self.circle(s(18.0), t(14.0), 1.2, w, true);
            }
            IconId::Terrain => {
                self.line(s(3.0), t(18.0), s(8.0), t(10.0), w);
                self.line(s(8.0), t(10.0), s(14.0), t(16.0), w);
                self.line(s(14.0), t(16.0), s(21.0), t(7.0), w);
            }
            IconId::VoxelTerrain => {
                self.rect_stroke(s(5.0), t(12.0), 6.0, 6.0, w);
                self.rect_stroke(s(11.0), t(8.0), 6.0, 6.0, w);
                self.rect_stroke(s(13.0), t(14.0), 6.0, 6.0, w);
            }
            IconId::EmptyEntity => self.circle(s(12.0), t(12.0), 7.0, w, false),
            IconId::Camera => {
                self.rect_stroke(s(4.0), t(8.0), 16.0, 10.0, w);
                self.circle(s(12.0), t(13.0), 3.5, w, false);
                self.rect_stroke(s(7.0), t(6.0), 4.0, 2.0, w);
            }
            IconId::Mesh => {
                self.line(s(12.0), t(4.0), s(20.0), t(18.0), w);
                self.line(s(20.0), t(18.0), s(4.0), t(18.0), w);
                self.line(s(4.0), t(18.0), s(12.0), t(4.0), w);
            }
            IconId::Texture => self.rect_stroke(s(5.0), t(5.0), 14.0, 14.0, w),
            IconId::Material => {
                self.circle(s(12.0), t(12.0), 8.0, w, false);
                self.line(s(6.0), t(12.0), s(18.0), t(12.0), w);
            }
            IconId::Scene => {
                self.rect_stroke(s(4.0), t(6.0), 16.0, 12.0, w);
                self.line(s(4.0), t(10.0), s(20.0), t(10.0), w);
            }
            IconId::Audio => {
                self.line(s(6.0), t(10.0), s(10.0), t(10.0), w);
                self.line(s(10.0), t(10.0), s(14.0), t(6.0), w);
                self.line(s(14.0), t(6.0), s(14.0), t(18.0), w);
                self.circle(s(18.0), t(10.0), 3.0, w, false);
            }
            IconId::Shader => {
                self.line(s(8.0), t(5.0), s(5.0), t(12.0), w);
                self.line(s(5.0), t(12.0), s(10.0), t(12.0), w);
                self.line(s(10.0), t(12.0), s(7.0), t(19.0), w);
                self.line(s(16.0), t(5.0), s(19.0), t(19.0), w);
            }
            IconId::Font => {
                self.line(s(6.0), t(18.0), s(12.0), t(6.0), w);
                self.line(s(12.0), t(6.0), s(18.0), t(18.0), w);
                self.line(s(8.5), t(14.0), s(15.5), t(14.0), w);
            }
            IconId::Script => {
                self.rect_stroke(s(6.0), t(4.0), 12.0, 16.0, w);
                self.line(s(9.0), t(8.0), s(15.0), t(8.0), w);
                self.line(s(9.0), t(12.0), s(15.0), t(12.0), w);
            }
            IconId::Json => {
                self.line(s(8.0), t(5.0), s(6.0), t(12.0), w);
                self.line(s(6.0), t(12.0), s(8.0), t(19.0), w);
                self.line(s(16.0), t(5.0), s(18.0), t(12.0), w);
                self.line(s(18.0), t(12.0), s(16.0), t(19.0), w);
            }
            IconId::License => self.rect_stroke(s(6.0), t(4.0), 12.0, 16.0, w),
            IconId::Unknown => {
                self.rect_stroke(s(7.0), t(4.0), 10.0, 16.0, w);
                self.line(s(10.0), t(8.0), s(14.0), t(8.0), w);
            }
            IconId::Derived => {
                self.rect_stroke(s(5.0), t(5.0), 14.0, 14.0, w);
                self.line(s(8.0), t(12.0), s(16.0), t(12.0), w);
            }
            IconId::Transform => {
                self.line(s(12.0), t(4.0), s(12.0), t(20.0), w);
                self.line(s(4.0), t(12.0), s(20.0), t(12.0), w);
            }
            IconId::Light => {
                self.circle(s(12.0), t(10.0), 5.0, w, false);
                self.line(s(12.0), t(15.0), s(12.0), t(20.0), w);
            }
            IconId::PostFx => {
                self.rect_stroke(s(4.0), t(6.0), 16.0, 12.0, w);
                self.circle(s(10.0), t(12.0), 3.0, w, false);
                self.circle(s(15.0), t(12.0), 3.0, w, false);
            }
            IconId::Water => {
                self.line(s(4.0), t(10.0), s(8.0), t(14.0), w);
                self.line(s(8.0), t(14.0), s(12.0), t(10.0), w);
                self.line(s(12.0), t(10.0), s(16.0), t(14.0), w);
                self.line(s(16.0), t(14.0), s(20.0), t(10.0), w);
                self.line(s(4.0), t(16.0), s(20.0), t(16.0), w);
            }
            IconId::Vessel => {
                self.line(s(4.0), t(14.0), s(20.0), t(14.0), w);
                self.line(s(4.0), t(14.0), s(8.0), t(18.0), w);
                self.line(s(20.0), t(14.0), s(16.0), t(18.0), w);
                self.line(s(8.0), t(18.0), s(16.0), t(18.0), w);
                self.line(s(12.0), t(14.0), s(12.0), t(6.0), w);
            }
            IconId::Ok => {
                self.circle(s(12.0), t(12.0), 8.0, w, false);
                self.line(s(8.0), t(12.0), s(11.0), t(16.0), w);
                self.line(s(11.0), t(16.0), s(17.0), t(8.0), w);
            }
            IconId::Warn => {
                self.line(s(12.0), t(5.0), s(4.0), t(19.0), w);
                self.line(s(12.0), t(5.0), s(20.0), t(19.0), w);
                self.line(s(4.0), t(19.0), s(20.0), t(19.0), w);
                self.line(s(12.0), t(10.0), s(12.0), t(14.0), w);
                self.circle(s(12.0), t(16.5), 1.0, w, true);
            }
            IconId::Error => {
                self.circle(s(12.0), t(12.0), 8.0, w, false);
                self.line(s(8.0), t(8.0), s(16.0), t(16.0), w);
                self.line(s(16.0), t(8.0), s(8.0), t(16.0), w);
            }
            IconId::Check => {
                self.line(s(6.0), t(12.0), s(10.0), t(17.0), w);
                self.line(s(10.0), t(17.0), s(18.0), t(7.0), w);
            }
            IconId::Minimize => {
                self.line(s(6.0), t(16.0), s(18.0), t(16.0), w + 0.6);
            }
            IconId::Maximize => {
                self.rect_stroke(s(6.0), t(6.0), 12.0, 12.0, w);
            }
            IconId::ImmersivePlay => {
                self.line(s(4.0), t(4.0), s(9.0), t(4.0), w);
                self.line(s(4.0), t(4.0), s(4.0), t(9.0), w);
                self.line(s(20.0), t(4.0), s(15.0), t(4.0), w);
                self.line(s(20.0), t(4.0), s(20.0), t(9.0), w);
                self.line(s(4.0), t(20.0), s(9.0), t(20.0), w);
                self.line(s(4.0), t(20.0), s(4.0), t(15.0), w);
                self.line(s(20.0), t(20.0), s(15.0), t(20.0), w);
                self.line(s(20.0), t(20.0), s(20.0), t(15.0), w);
                self.line(s(9.0), t(8.0), s(9.0), t(16.0), w);
                self.line(s(9.0), t(8.0), s(17.0), t(12.0), w);
                self.line(s(9.0), t(16.0), s(17.0), t(12.0), w);
            }
            IconId::SculptRaise | IconId::SculptLower => {
                self.line(s(3.0), t(20.0), s(21.0), t(20.0), w);
                self.polyline(
                    &[(s(6.5), t(16.5)), (s(12.0), t(10.0)), (s(17.5), t(16.5))],
                    w,
                );
                if id == IconId::SculptRaise {
                    self.line(s(12.0), t(7.0), s(12.0), t(2.5), w);
                    self.polyline(&[(s(9.6), t(4.9)), (s(12.0), t(2.5)), (s(14.4), t(4.9))], w);
                } else {
                    self.line(s(12.0), t(2.5), s(12.0), t(7.0), w);
                    self.polyline(&[(s(9.6), t(4.6)), (s(12.0), t(7.0)), (s(14.4), t(4.6))], w);
                }
            }
            IconId::SculptSmooth => {
                self.line(s(3.0), t(20.0), s(21.0), t(20.0), w);
                self.polyline(
                    &[
                        (s(3.0), t(14.0)),
                        (s(5.0), t(13.5)),
                        (s(7.0), t(9.0)),
                        (s(9.4), t(8.0)),
                        (s(12.0), t(10.5)),
                        (s(15.8), t(14.0)),
                        (s(18.5), t(12.0)),
                        (s(21.0), t(11.0)),
                    ],
                    w,
                );
            }
            IconId::SculptFlatten => {
                self.line(s(3.0), t(20.0), s(21.0), t(20.0), w);
                self.line(s(3.0), t(9.0), s(21.0), t(9.0), w);
                self.polyline(
                    &[
                        (s(6.0), t(15.0)),
                        (s(9.0), t(12.0)),
                        (s(12.0), t(11.0)),
                        (s(15.0), t(12.0)),
                        (s(18.0), t(15.0)),
                    ],
                    w,
                );
            }
            IconId::SculptNoise => {
                self.line(s(3.0), t(20.0), s(21.0), t(20.0), w);
                self.polyline(
                    &[
                        (s(3.0), t(15.0)),
                        (s(5.4), t(10.0)),
                        (s(7.8), t(14.0)),
                        (s(10.2), t(7.0)),
                        (s(12.6), t(15.0)),
                        (s(15.0), t(10.0)),
                        (s(17.4), t(13.0)),
                        (s(21.0), t(11.0)),
                    ],
                    w,
                );
            }
            IconId::PaintLayer => {
                self.polyline(
                    &[
                        (s(12.0), t(3.0)),
                        (s(3.0), t(8.0)),
                        (s(12.0), t(13.0)),
                        (s(21.0), t(8.0)),
                        (s(12.0), t(3.0)),
                    ],
                    w,
                );
                self.polyline(
                    &[(s(3.0), t(14.0)), (s(12.0), t(19.0)), (s(21.0), t(14.0))],
                    w,
                );
            }
            IconId::FoliagePaint | IconId::FoliageErase => {
                self.arc(s(12.0), t(10.0), 7.0, 165.0, 355.0, w);
                self.polyline(
                    &[
                        (s(5.0), t(16.0)),
                        (s(5.0), t(12.0)),
                        (s(10.0), t(6.0)),
                        (s(20.0), t(4.0)),
                    ],
                    w,
                );
                if id == IconId::FoliagePaint {
                    self.polyline(
                        &[(s(4.0), t(21.0)), (s(8.0), t(15.0)), (s(13.0), t(12.0))],
                        w,
                    );
                } else {
                    self.line(s(3.0), t(21.0), s(21.0), t(3.0), w);
                }
            }
            IconId::FoliageSingle => {
                self.line(s(12.0), t(21.0), s(12.0), t(9.0), w);
                self.polyline(
                    &[
                        (s(12.0), t(12.0)),
                        (s(15.0), t(6.0)),
                        (s(18.5), t(5.0)),
                        (s(18.0), t(9.0)),
                        (s(12.0), t(12.0)),
                    ],
                    w,
                );
                self.polyline(
                    &[
                        (s(12.0), t(15.0)),
                        (s(9.0), t(10.0)),
                        (s(6.5), t(9.0)),
                        (s(7.0), t(12.5)),
                        (s(12.0), t(15.0)),
                    ],
                    w,
                );
            }
            IconId::LightProbe => {
                self.circle(s(12.0), t(12.0), 8.5, w, false);
                self.arc(s(9.0), t(12.0), 6.0, -67.0, 67.0, w);
                self.line(s(3.5), t(12.0), s(20.5), t(12.0), w);
            }
            IconId::MaterialGraph => {
                self.rect_stroke(s(2.0), t(4.0), 7.0, 6.0, w);
                self.rect_stroke(s(15.0), t(14.0), 7.0, 6.0, w);
                self.polyline(
                    &[
                        (s(9.0), t(7.0)),
                        (s(12.5), t(7.0)),
                        (s(14.5), t(9.0)),
                        (s(14.5), t(15.5)),
                    ],
                    w,
                );
                self.line(s(2.0), t(17.0), s(8.0), t(17.0), w);
            }
            IconId::RayTrace => {
                self.line(s(2.0), t(5.0), s(7.0), t(5.0), w);
                self.polyline(
                    &[(s(4.5), t(5.0)), (s(13.5), t(12.0)), (s(10.5), t(20.0))],
                    w,
                );
                self.line(s(13.5), t(12.0), s(20.5), t(6.0), w);
                self.circle(s(21.0), t(4.5), 1.6, w, false);
                self.line(s(3.0), t(20.0), s(8.0), t(20.0), w);
            }
        }
    }
}

impl Default for IconAtlas {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_covers_every_icon() {
        let atlas = IconAtlas::new();
        assert_eq!(atlas.width, ICON_ATLAS_WIDTH);
        let mut painted = 0u32;
        for px in atlas.pixels.chunks(4) {
            if px[3] > 0 {
                painted += 1;
            }
        }
        assert!(painted > 1000, "atlas should contain stroked glyphs");
        let (uv0, uv1) = IconId::Play.uv_rect();
        assert!(uv1.x > uv0.x && uv1.y > uv0.y);
    }

    #[test]
    fn zeta_extension_icons_have_visible_coverage() {
        let atlas = IconAtlas::new();
        for id in [
            IconId::SculptRaise,
            IconId::SculptLower,
            IconId::SculptSmooth,
            IconId::SculptFlatten,
            IconId::SculptNoise,
            IconId::PaintLayer,
            IconId::FoliagePaint,
            IconId::FoliageErase,
            IconId::FoliageSingle,
            IconId::LightProbe,
            IconId::MaterialGraph,
            IconId::RayTrace,
        ] {
            let (ox, oy) = IconAtlas::cell_origin(id);
            let mut painted = 0usize;
            for y in oy as u32..oy as u32 + ICON_CELL {
                for x in ox as u32..ox as u32 + ICON_CELL {
                    let alpha = atlas.pixels[((y * atlas.width + x) * 4 + 3) as usize];
                    painted += usize::from(alpha > 0);
                }
            }
            assert!(painted > 20, "{id:?} should paint its atlas cell");
        }
    }
}

#[cfg(test)]
mod large_cut_tests {
    use super::*;
    use crate::theme;

    #[test]
    fn both_cuts_fit_the_atlas_and_never_overlap() {
        for &id in IconId::ALL {
            let (sx, sy) = id.cell_px();
            assert!(sx + ICON_CELL <= ICON_ATLAS_WIDTH, "{id:?} small cut x");
            assert!(
                sy + ICON_CELL <= LARGE_ORIGIN_Y,
                "{id:?} small cut spills into the large block"
            );
            let (lx, ly) = id.cell_px_large();
            assert!(
                lx + ICON_CELL_LARGE <= ICON_ATLAS_WIDTH,
                "{id:?} large cut x"
            );
            assert!(
                ly + ICON_CELL_LARGE <= ICON_ATLAS_HEIGHT,
                "{id:?} large cut y"
            );
            assert!(
                ly >= LARGE_ORIGIN_Y,
                "{id:?} large cut overlaps the small block"
            );
        }
    }

    #[test]
    fn a_drawer_tile_samples_the_large_cut_and_a_row_icon_the_small_one() {
        let row = IconId::Folder.draw_quad(Rect::new(0.0, 0.0, 16.0, 16.0));
        let tile =
            IconId::Folder.draw_quad(Rect::new(0.0, 0.0, theme::ICON_DRAWER, theme::ICON_DRAWER));
        assert_ne!(row.0[0], tile.0[0], "both sizes sampled the same cell");
        let (small, _) = IconId::Folder.uv_rect();
        let (large, _) = IconId::Folder.uv_rect_large();
        assert_eq!(row.0[0], small);
        assert_eq!(tile.0[0], large);
    }

    #[test]
    fn the_large_cut_carries_real_coverage() {
        // Guards the case where the large block is allocated but never filled,
        // which would show as invisible drawer tiles rather than pixelated ones.
        let atlas = IconAtlas::new();
        let (ox, oy) = IconId::Folder.cell_px_large();
        let mut covered = 0;
        for y in 0..ICON_CELL_LARGE {
            for x in 0..ICON_CELL_LARGE {
                let i = (((oy + y) * atlas.width + (ox + x)) * 4 + 3) as usize;
                if atlas.pixels[i] > 8 {
                    covered += 1;
                }
            }
        }
        assert!(covered > 200, "large Folder cut has only {covered} pixels");
    }
}
