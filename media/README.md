# Media

Screenshots and GIFs used by the top-level [`README.md`](../README.md).

Drop image files here and they'll show up in the README's **Screenshots**
section. Suggested captures:

| Filename | What to show |
|---|---|
| `editor.png` | ✅ **done** — command scopes, viewport, Outliner/Details, docked Content Drawer |
| `terrain.png` | Heightmap terrain with splatmap painting / sculpted hills |
| `voxel.png` | The voxel world streaming around the camera |
| `shadows.png` | Cascaded shadows + PBR materials on the glTF scene |
| `water.png` | Great Lakes water with RT reflections (Help → Water) |

Tips:
- PNG for stills, GIF (or MP4 linked, not embedded) for motion.
- Keep individual files reasonably small (a few MB) so the repo stays light —
  downscale 4K captures to ~1600px wide.
- The README references these by path, so keep the filenames above (or update the
  links in `README.md` if you rename them).

## Capturing

Screenshots come from the engine, not from a screen grabber, so they are
deterministic and always show a real build:

```sh
SOMNIUM_CAPTURE_UI_PNG=media/editor.png \
SOMNIUM_CAPTURE_FRAME=140 \
SOMNIUM_CAPTURE_QUIT=1 \
SOMNIUM_TERRAIN=1 \
cargo run -p hello_engine
```

`SOMNIUM_CAPTURE_UI_PNG` reads the swapchain back **after** the UI pass, so it
includes editor chrome. `SOMNIUM_CAPTURE_DISPLAY_PNG` runs *before* it, on
purpose, so a scene A/B measures the render rather than the panels.
