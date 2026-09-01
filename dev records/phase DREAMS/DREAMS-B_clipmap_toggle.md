# DREAMS-B amendment — the Clipmap checkbox had not worked since 439b6b6

Reported as "these artifacts are back after DREAMS B", then narrowed by the
user to "whenever i toggle either stf or grain, clipmap is toggled on by it",
and finally to "i just toggled off grain and clipmap STILL toggled on" after a
first fix that addressed only two of the three writers.

DREAMS-B did not cause this. It made a five-month-old bug reachable.

## Three writers of one bool

```
                      TerrainClipmap::enabled
                               ^
            +------------------+------------------+
            |                  |                  |
  apply_debug_toggles   ToggleTerrainClipmap   submit_terrains
  (renderer.rs)         (app.rs, on click)     (app.rs, EVERY FRAME)
  from debug_toggles    cm.enabled = !cm...    cm.enabled = true
                                                if virtual_texture_enabled
```

Ordering decides the winner, and the per-frame writer runs last and always.

- `debug_toggles["terrain_clipmap"]` defaults **on**.
- `TerrainClipmap::new` defaults **off** (`env_default_enabled`, "off until DF-E
  gates pass"). The two defaults already disagreed.
- `submit_terrains` force-enabled it for any terrain with
  `virtual_texture_enabled`, on every frame.
- `TerrainData::virtual_texture_enabled` is `physical_capacity > 0`.
  `configure_virtual_texture` takes `_enabled: bool` and ignores it, so VT
  cannot be switched off once the atlas exists.
- The atlas exists whenever `bc_supported && bc7_packs_complete() &&
  !force_rgba8()` — true on the reporter's machine.

Therefore, on any BC7-capable machine, the clipmap was **on and unturnoffable**
from commit `439b6b6` onward. The click was accepted, reverted inside the same
frame, and the checkbox re-ticked itself at the next inspector refresh, since
`v.clipmap` is read back from live state.

## Why it looked like DREAMS-B

Nothing called `apply_debug_toggles` at runtime before DREAMS-B. Its two
switches (`dreams_grain`, `dreams_stf`) were the first, so a DREAMS click was
the first thing that ever forced a visible re-sync of the terrain switches —
including a checkbox that had been lying since April. The correlation was real
and the causation was not.

## The requirement underneath is real

`load_bc7_layers` registers 4x4 placeholders for the legacy layer arrays on
purpose (`textures.rs:1482`): in VT mode the real BC7 pages reach shading only
through the clipmap rings. A VT terrain with the clipmap off is not "the
clipmap turned off" — it is terrain shaded from eight mean colours. The
per-frame force-enable was defending something true.

It was only wrong in *how*: silently, from outside the renderer, overriding the
control that claimed to own the setting. Silent enforcement and a broken
checkbox are indistinguishable from the outside, which is why this survived
three prior investigations.

## Fix

One writer, `SomniumRenderer::reconcile_clipmaps`, holding both the switch and
the constraint:

```rust
let want = !TerrainClipmap::env_forced_off()
    && (self.debug_toggles.is_on("terrain_clipmap")
        || self.clipmap_owned_by_virtual_texturing());
```

`submit_terrains` calls it once per frame instead of assigning. The clipmap
switch now *refuses* the off-click under VT and says why, through the same
`push_toast` path the environment override already used.

Invalidate-on-enable moved in with it: coming back on with rings still centred
where the camera used to be is itself a straight-edged patch of wrong terrain.

## Guards

| Test | Catches |
|---|---|
| `the_frame_loop_does_not_own_whether_the_clipmap_is_on` | a fourth writer appearing in `submit_terrains` |
| `the_clipmap_cannot_be_switched_off_out_from_under_virtual_texturing` | the switch accepting an off-click it cannot honour |
| `terrain_switches_are_flipped_through_the_toggle_state` | any terrain arm going back to direct assignment |

The first reads code with comment lines stripped: the comment explaining the
rule quotes the line the rule forbids.

## Left open

`DF-OPEN_clipmap_band_artifact.md` is still open. This changes only whether the
clipmap can be turned off; the bands it produces when on are unexplained, and
no fourth attribution is recorded here.

The detail rings reach ~128 m and centre on a focus point derived from a 1.7 m
eye height looking <=8 m ahead. That is a walking design, and the reports are
from flight. Noted as a lead, not a diagnosis.
