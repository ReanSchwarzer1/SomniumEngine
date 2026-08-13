# Content Drawer

The drawer is the project file browser. It stays docked above the status bar, like a Content Browser.

## Opening it

- Click **Content Drawer** on the bottom-left tray (icon + name).
- Or press **Ctrl+Space**.
- Click **Content Drawer** again to hide it. **Output Log** swaps in the same slot.

## What you see

- Default root is project **assets/**, shown as **Game**.
- Folders and files are large tiles in a row that wraps (80 px glyphs). Double-click a folder to enter it. Click the breadcrumb to walk back.
- **Show Engine Content** reveals virtual primitives (Cube, Sphere, lights) under Engine.
- Derived packs under **assets/terrain/bc7/** stay hidden.

## Import

**File → Import Model** still opens a native picker for glTF/GLB. Imported nodes appear in the Outliner and can be selected immediately.

New asset kinds (cooked packs, animation clips, prefabs, …) will show up here as those systems land. The drawer is not a finished catalog. Lighting extras (world cache, path tracer, area lights) are Details / Create-menu controls, not drawer assets.
