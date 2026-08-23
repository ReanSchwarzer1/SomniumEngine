# CONTROL-A capture manifest

CONTROL-A records 14 deterministic editor states at both required logical
window sizes. All 28 PNGs were emitted directly by the UI-inclusive swapchain
capture; none was resized, composited, or invented. A PNG-header verification
on 2026-08-23 confirmed every `*_1280x720.png` is exactly 1280x720 and every
`*_1920x1080.png` is exactly 1920x1080.

| Surface | State / artifact stem | 1280x720 | 1920x1080 |
|---|---|---:|---:|
| Default editor shell | `shell` | captured | captured |
| File, Edit, Create, View, Window, Help menus | `menu-file`, `menu-edit`, `menu-create`, `menu-view`, `menu-window`, `menu-help` | captured | captured |
| Command palette | `palette` | captured | captured |
| Help overlay | `help` | captured | captured |
| Output Log panel | `log` | captured | captured |
| Profiler overlay | `profiler` | captured | captured |
| Unsaved-scene modal | `modal-unsaved` | captured | captured |
| Populated Details (`Post Processing`) | `details-post` | captured | captured |
| Content Drawer at the real `assets/terrain/` folder | `drawer-terrain` | captured | captured |

The original `editor_1280x720.png` is retained as the pre-driver shell capture.
Visual inspection covered the default shell, File menu, unsaved modal,
populated Details, and terrain Drawer at 1920x1080. The modal uses the shipped
unsaved-scene prompt; Details selects the already-spawned `Post Processing`
entity by name; the Drawer reads the real folder and thumbnails.

The startup driver is deliberately diagnostic-only:

- `SOMNIUM_AUDIT_WINDOW_SIZE=WIDTHxHEIGHT` selects the initial logical size.
- `SOMNIUM_AUDIT_UI_STATE` accepts `shell`, each `menu-*` state, `palette`,
  `help`, `log`, `profiler`, or `modal-unsaved`.
- `SOMNIUM_AUDIT_SELECT_ENTITY=Post Processing` selects an existing entity.
- `SOMNIUM_AUDIT_CONTENT_PATH=terrain` opens a validated relative directory
  below `assets/` before the first content refresh.

Representative capture command:

```powershell
cargo build -p hello_engine -j 1
$env:SOMNIUM_AUDIT_WINDOW_SIZE='1920x1080'
$env:SOMNIUM_AUDIT_UI_STATE='modal-unsaved'
$env:SOMNIUM_TERRAIN='none'
$env:SOMNIUM_CAPTURE_UI_PNG="$PWD/dev records/phase CONTROL/CONTROL-A_baseline/modal-unsaved_1920x1080.png"
$env:SOMNIUM_CAPTURE_FRAME='10'
$env:SOMNIUM_CAPTURE_QUIT='1'
& "$PWD/target/debug/hello_engine.exe"
```

Residual: property-target transient widgets (the colour picker, combo drop-downs,
and target-specific context menus) do not have a standalone audit state. Opening
them without an active property/edit transaction would manufacture a target,
so they are not claimed as CONTROL-A surface baselines. The persistent editor
regions, all six application menus, global overlays, a real modal, and the
required alternate Details/Drawer states are covered above.
