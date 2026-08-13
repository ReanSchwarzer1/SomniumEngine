//! Original geometric icon atlas for Metaphor chrome (Phase 26-A).
//!
//! Stroke-icon optical sizes follow Lucide (16/20/32) without copying Lucide
//! SVG path data. Engine mark is an original crescent + S. See ATTRIBUTION.

use glam::Vec2;

use crate::types::Rect;

/// wgpu texture_id reserved for the icon atlas.
pub const ICON_ATLAS_TEXTURE_ID: u32 = 1;

pub const ICON_CELL: u32 = 32;
pub const ICON_ATLAS_WIDTH: u32 = 512;
pub const ICON_ATLAS_HEIGHT: u32 = 512;
const CELLS_PER_ROW: u32 = ICON_ATLAS_WIDTH / ICON_CELL;

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
    ];

    pub fn index(self) -> u32 {
        self as u32
    }

    pub fn uv_rect(self) -> (Vec2, Vec2) {
        let i = self.index();
        let col = i % CELLS_PER_ROW;
        let row = i / CELLS_PER_ROW;
        let u0 = col as f32 * ICON_CELL as f32 / ICON_ATLAS_WIDTH as f32;
        let v0 = row as f32 * ICON_CELL as f32 / ICON_ATLAS_HEIGHT as f32;
        let du = ICON_CELL as f32 / ICON_ATLAS_WIDTH as f32;
        let dv = ICON_CELL as f32 / ICON_ATLAS_HEIGHT as f32;
        (Vec2::new(u0, v0), Vec2::new(u0 + du, v0 + dv))
    }

    pub fn draw_quad(self, _dest: Rect) -> ([Vec2; 4], u32) {
        let (uv0, uv1) = self.uv_rect();
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
            atlas.rasterize(id);
        }
        atlas
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

    fn punch(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        if x >= self.width || y >= self.height {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        self.pixels[i + 3] = 0;
    }

    fn erase_circle(&mut self, cx: f32, cy: f32, r: f32) {
        let minx = (cx - r - 1.0).floor() as i32;
        let maxx = (cx + r + 1.0).ceil() as i32;
        let miny = (cy - r - 1.0).floor() as i32;
        let maxy = (cy + r + 1.0).ceil() as i32;
        for y in miny..=maxy {
            for x in minx..=maxx {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                if (dx * dx + dy * dy).sqrt() <= r {
                    self.punch(x, y);
                }
            }
        }
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
                // Filled crescent — readable at 24–32 px, unlike a 1.8 px stroke.
                self.circle(s(12.0), t(12.0), 10.0, 1.0, true);
                self.erase_circle(s(16.5), t(10.0), 7.2);
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
        assert_eq!(atlas.width, 512);
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
}
