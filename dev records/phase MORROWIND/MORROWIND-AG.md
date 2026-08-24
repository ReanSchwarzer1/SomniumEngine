# MORROWIND-AG — audio, from 93 lines to a crate

**Items 1, 2, 4 and 5 complete, 2026-08-24. Items 3 and 6 are scoped and named
below.** Track 8 (ALMSIVI). §8 calls this *"the cheapest large win in the
phase"*, and it was.

## What was there

```
bus.rs        1 line:  // Bus stub
error.rs      1 line:  // Error stub
listener.rs   1 line:  // Listener stub
engine.rs    49 lines
sound.rs     36 lines
lib.rs        5 lines
```

93 lines, **zero tests**, one caller. No bus, no listener, no spatialisation, no
attenuation, no cache. A file played twice was read twice.

## The bug this sub-phase is named after

```rust
let _kira_settings = StaticSoundSettings::new().volume(settings.volume);
let sound_data = StaticSoundData::from_file(path)?;
let handle = self.manager.play(sound_data)?;
```

The settings were built into an underscore-prefixed variable **and then not
used**. Every sound played at full volume; the `volume` argument did nothing,
silently, for as long as the crate has existed.

§8 item 5 asks for the fix *and* the test, and says why it is worth more than a
one-line diff:

> *"a one-line fix and **a permanent lesson about the second-example rule**."*

The lesson, stated plainly: **`somnium_audio` had one caller and zero tests.**
Nothing ever asked it to be quieter, so nothing noticed that it could not be.
`applying_a_volume_actually_changes_the_gain` is that test, and it needs no audio
device — which is the other half of it. The check was cheap and simply absent.

Fixing it turned up a second dropped field on the same line: **`looping` was
also being discarded.** Same cause, same test file, one more assertion.

## Buses (item 1)

Master, Music, SFX, Dialogue, UI, as a **tree**, because "quieter dialogue" and
"quieter everything" are different sliders and the second has to affect the
first. A flat list makes master volume a multiplication every caller has to
remember to do.

The gain graph is resolved in `bus.rs` rather than pushed into Kira's tracks,
and the reason is **solo**. Solo is not a property of a bus, it is a property of
the mixer — soloing one bus silences every bus not on a soloed path — so
expressing it as per-track volumes means recomputing every track whenever any
bus changes. One function, one test.

Three rules that only show up under use:

- **A soloed ancestor keeps its children.** Otherwise solo is useless on any
  tree deeper than one level: soloing Music would silence Music's own sub-buses.
- **Mute beats solo** on the same bus. Mute is the explicit "silence this"; solo
  is "silence the others".
- **A negative volume is silence, not a phase inversion.** A negative gain flips
  the waveform, which is inaudible alone and *cancels* the sound when it mixes
  with anything correlated — a bug that appears only when two things play at
  once.

Plus: a sound routed to a typo is silent and findable rather than full volume on
master, and a parent cycle terminates rather than spinning (a cycle is reachable
by editing a settings file).

## The listener and spatialisation (item 2)

`Listener` with position, orientation and velocity; `Emitter` with attenuation,
a cone and an occlusion factor; `evaluate` producing gain, pan, Doppler and
distance.

**Attenuation curves are an enum with an authored variant**, so CONTROL-K's
curve editor plugs in as `Attenuation::Curve(points)` rather than as a different
code path. `InverseSquare` is the default because it is what ears expect;
`Linear` is kept because it is what a UI wants and what a designer reaches for
when they want predictability.

Two details in the physical curve that matter:

- `min / d` never reaches zero, so it is faded out over the last stretch before
  `max`. Without that a distant source **cuts out abruptly** instead of receding,
  and the cut is audible.
- An authored `min` above `max` is corrected rather than dividing by a negative
  range — which would produce gains above one, i.e. **a sound that gets louder
  with distance**.

`occlusion` is a plain factor set by whoever queries the physics world, *not*
computed here. A sound system that cannot be tested without a physics world is a
sound system nobody tests.

### The second bug: Doppler had the wrong sign

The first version used the same direction vector for both halves of
`f' = f(c + v_r)/(c - v_s)`. But `v_r` is the *listener* moving toward the
source and `v_s` is the *source* moving toward the listener — opposite
directions along the same line. With one sign wrong, **every approaching sound
dropped in pitch**: the exact opposite of the effect, and the sort of error that
sounds "a bit off" rather than obviously broken.

`doppler_shifts_the_right_way` caught it. `a_moving_listener_shifts_too` was
added because the two halves have different signs, so testing only the emitter
half would leave the listener half free to be wrong.

And a supersonic source is clamped rather than divided: the formula has a pole
at the speed of sound and goes negative past it, and an infinite or negative
playback rate is a crash in most resamplers. A scripted object moving that fast
is a thing designers do.

## The cache (item 4)

`Sounds` keys decoded `StaticSoundData` by path. `StaticSoundData` is internally
reference-counted, so a cached entry costs a clone per play rather than a
decode. A missing file is a **distinct error** from a corrupt one, because the
causes differ — a typo or an asset step that did not run, versus a bad file —
and telling them apart in the log saves the wrong investigation.

A failed load caches nothing, so a missing file does not poison the entry.

## Errors (item 3's file)

`error.rs` was `// Error stub`. It now holds the one judgement worth writing
once: **no audio failure is fatal.** A missing file is a content bug and the
right response is a log line and silence for that sound; no audio device is an
environment fact — a CI machine, a container, a player with no sound card — and
everything else must keep working. A game that will not start because one
footstep is missing is worse than one that is quiet.

## What is not here, and why

- **Reverb zones and occlusion queries against the physics world** (item 3's
  other half). The *hook* is here — `Emitter::occlusion` — and it is a plain
  factor on purpose. Doing the raycast inside this crate would add a
  `somnium_physics` dependency and make the whole crate untestable without a
  physics world, which is how the 93-line version got to zero tests. The caller
  that owns both is `somnium_core`, and that is where the query belongs.
- **A mixer panel and audio tracks in MORROWIND-L's timeline** (item 6).
  MORROWIND-L does not exist; the timeline is Track 2. `Mixer` is public and
  drivable by an options screen today, which is the half that does not depend on
  a sub-phase that has not started.

## Tests: 40 new, 0 failures — from zero

- **`bus`, 11** — the default buses; master multiplying down the chain; siblings independent; muting a parent; **solo silencing the rest**; a soloed ancestor keeping its children; mute beating solo; a negative volume clamping; an unknown bus silent; a parent cycle terminating.
- **`listener`, 16** — full inside `min`, silent past `max`; inverse-square falling off faster than linear; **both curves monotonic**; an inverted range corrected; authored curves interpolating; an empty curve audible rather than silent; **panning following orientation**; turning swapping the ears; a source at the listener centred; cones; occlusion; **Doppler in the right direction**, for a moving emitter *and* a moving listener; perpendicular motion not shifting; a supersonic source clamped.
- **`engine`, 11** — **the volume actually reaching the sound**; bus gain multiplying in; negative clamping; **looping reaching the settings**; panning mapped into Kira's `0..=1` range; spatial gain multiplying; a missing file named; a failed load caching nothing; an inaudible emitter not an error.
- **`error`, 2** — no audio failure fatal; errors naming what failed.

## Files

```
~ crates/somnium_audio/src/bus.rs        1 line -> Mixer, Bus, the gain graph
~ crates/somnium_audio/src/listener.rs   1 line -> Listener, Attenuation, Cone,
                                          Emitter, evaluate, Doppler
~ crates/somnium_audio/src/error.rs      1 line -> is_fatal, and the reasoning
~ crates/somnium_audio/src/engine.rs     the volume fix, Sounds, play_on,
                                          play_spatial, kira_settings
```
