# Content Drawer

The drawer is the project file browser. It stays docked above the status bar, like a Content Browser.

## Opening it

- Click **Content Drawer** on the bottom-left tray (icon + name).
- Or press **Ctrl+Space**.
- Click **Content Drawer** again to hide it. **Output Log** swaps in the same slot.

## What you see

- Default root is project **assets/**, shown as **Game**.
- Folders and files are large tiles in a row that wraps (80 px glyphs). Click a folder to enter it. Click the breadcrumb to walk back.
- **Maps** live under **Game / Maps**. Double-click a `.somnium` map to load it (Unreal Content Browser style). **Coastal** is the launch landscape (1 km, 32-layer Appalachia). **Island** is a compact FBM island in a 512 m ocean tile (16-layer hero bank; GPU splat format stays 32 slots with 16–31 empty). Same water look as Coastal.
- **Show Engine Content** reveals virtual primitives (Cube, Sphere, lights) under Engine. Single-click a primitive to spawn it.
- Derived packs under **assets/terrain/bc7/** stay hidden.

## Import

**File → Import Model** still opens a native picker for glTF/GLB. Imported nodes appear in the Outliner and can be selected immediately.

New asset kinds (cooked packs, animation clips, prefabs, …) will show up here as those systems land. The drawer is not a finished catalog. Lighting extras (world cache, path tracer, area lights) are Details / Create-menu controls, not drawer assets.
