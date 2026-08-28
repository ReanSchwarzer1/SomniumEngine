# MORROWIND-I — accessibility

**Complete, 2026-08-25. Track 1 (VIVEC) closes with it.** §8: *"The row nobody
plans and everybody eventually needs. An accessibility tree mirroring the widget
tree, exposed to platform screen readers; focus and role announcements; a
respect-reduced-motion setting wired to MORROWIND-H; and a contrast mode that
reuses Zeta's certified pairs rather than inventing a second palette."*

All four, plus the platform adapter — which the plan scoped as *"the tree and
the reader integration"* and which turned out to be reachable rather than
speculative.

## Why a self-rendered UI has to do this at all

A UI drawn with the platform's own controls gets an accessibility tree for free,
because the controls *are* the tree. Somnium draws every pixel itself, so to a
screen reader the editor is **one opaque rectangle** — and so is every game
built on it.

§6.9.2 found that **Godot 4.5 shipped AccessKit support** for exactly this
reason, which is what moved this sub-phase from speculative to precedented: a
mainstream engine has now done this in a self-rendered UI and the hard
integration questions have public answers.

## The three hooks, and why not a registry

`Control` gained `role()`, `a11y_name()`, `a11y_value()` and `a11y_toggled()`,
all defaulted. The control already knows all four answers, and a registry
mapping widget types to roles would be a second place for them to be wrong.

A widget overriding none of them is presentational — the right default, because
a `Border` **is** presentational and so are most of the 28 widget types in the
crate. Twelve override: button, text, check box, slider, text box, image, scroll
viewer, tab control, tree view, toast, combo box, popup.

Two of those twelve are worth stating:

- **`ToastHost` is `Role::Alert`.** A toast is the reason `Politeness` exists:
  it appears without taking focus, so a reader that only speaks on focus change
  never mentions it, and the user never learns that the save failed.
- **`TreeView` is `Role::List`, not a tree role.** Somnium's outliner is the
  case, and `List` is what a reader can navigate with its list commands.

## The two things that are easy to get wrong

### 1. The name is not the debug name

`Widget::name` is what a developer called the node. The accessible name is what
a **user** would call it, and for an icon-only toolbar button the best available
answer is its **tooltip** — which the shell already authors, for the same reason
and without knowing it. `a11y_probe` falls back to it.

That is why `Button::a11y_name` returns `None` rather than an empty string: an
empty name would *shadow* the tooltip and every icon button in the shell would
read as "button".

### 2. A tree that mirrors the widget tree exactly is too deep

A button is a border containing a stack panel containing a text node. Reading
that to somebody as three nested groups is worse than useless.

`A11yTree::from_ui` **collapses**: a node with no role of its own, no name and no
value contributes its children directly instead of itself, and a `Group`
wrapping exactly one meaningful child is replaced by that child. The four-node
chain above becomes `button → label`, and the test asserts the tree has exactly
three nodes (window, button, label) rather than merely that the button survived.

A hidden subtree contributes **nothing**. To a screen reader an invisible widget
must not exist at all, or the reader reads out a menu that is closed.

## A test that passed by accident, and the comment now stopping it

The collapse test originally built its widgets children-first and wrapped them
afterwards. `UserInterface::add_node(node, parent)` pushes into the parent it is
*given* — so every node was also a direct child of the root, and the walk visited
the label twice. The test passed anyway.

The hidden-subtree test, built the same way, failed — and that failure is what
exposed the first one. Both are now built parent-first, the hidden-subtree test
gained a **visible sibling** so it cannot pass against an empty tree, and the
reason is a comment in the file.

## Announcements

Name, then role, then value, then state. That is the order every major screen
reader uses and the reason is that the name is what the user is looking for and
the rest is qualification:

```
"Cast shadows, check box, checked"
"Save, button, dimmed"
"button"                              ← unnamed: bad, but not silent
```

`unnamed()` reports **focusable** nodes with no name, and it is the single most
useful diagnostic in the module: an unnamed button is the most common real
accessibility failure in a self-rendered UI and is invisible to everyone who is
not using a reader. It is a list rather than a warning so a test can assert on
it. Labels are excluded — a nameless decoration is not a defect.

## Reduced motion and contrast, and the invariant that keeps them one product

```rust
ui.set_a11y_settings(A11ySettings { reduced_motion: true, high_contrast: true });
```

`reduced_motion` reaches MORROWIND-H's animator, which already implemented it —
this sub-phase's job was to make it reachable from a **setting** rather than only
from editor code. Both are `EditorSettings` fields with schema rows in the
`Accessibility` group, so CONTROL-B's property seam generates the toggles and a
game reads the same declaration. The platform is consulted first and the
preference ORs over it.

`A11ySettings::from_platform` returns defaults today, **stated rather than
hidden**: Windows exposes this through `SPI_GETCLIENTAREAANIMATION` and macOS
through `NSWorkspace`, and both are a platform-crate dependency this sub-phase
declined to add for one boolean. The function exists so the *call site* is
already right — the shell asks the platform and falls back to the setting — and
wiring the real query later changes one function body. There is a test asserting
it returns defaults, which will fail when somebody lands the real query and make
them update the doc comment with it.

**The invariant, asserted:** neither setting may move anything. There is a test
that lays out three buttons under all four combinations and compares bounds. A
high-contrast build that relaid out would be a second interface nobody tests.

### High contrast reuses Zeta rather than replacing it

Zeta's tokens are certified at specific ratios by `theme.rs`'s own contrast
tests. A high-contrast mode built from a *different* palette would be a second
palette with **no certification at all**, which is how a mode meant to help ends
up worse than the one it replaces.

So `high_contrast(fg, bg)` picks no new colours. It walks the existing
foreground toward whichever pole its background is not, by binary search, until
the ratio clears **7:1 — WCAG AAA for body text**. Zeta certifies at the normal
bar, which is right as a default; the point of a high-contrast mode is the users
for whom that bar is not enough, so a mode that merely re-achieved the default
would be a switch that does nothing.

Three properties, each tested: it never *lowers* a ratio; it preserves polarity,
so light-on-dark stays light-on-dark and the mode cannot invert one pair and not
another; and it keeps alpha, so a translucent wash stays a wash rather than
becoming a block of colour over the thing it was washing. A colour that already
clears the bar is returned unchanged, so the mode is a no-op on the parts Zeta
already got right.

## AccessKit, and the platform adapter

`accesskit` 0.24.1 in `somnium_ui` for the model; `accesskit_winit` 0.33.2 in
`somnium_core` for the platform. **`accesskit_winit` 0.33.2 requires
`winit ^0.30.5` and Somnium is on 0.30.13**, so this needed no windowing bump —
which was the objection this sub-phase expected to have to make.

`Role::to_accesskit` is a total `match` with no wildcard: adding a Somnium role
is a compile error here rather than a silent `GenericContainer` that a reader
announces as nothing. The role set was chosen to map 1:1 and it does.

One AccessKit rule that is easy to get backwards and has a test: **a `Label` node
carries its text in `value`, not `label`.** Getting it the other way makes static
text read as an empty label.

`to_accesskit` sends a **full tree every time** rather than a diff. Somnium's
trees are in the hundreds of nodes and the shell rebuilds widgets freely; a diff
would be an optimisation whose correctness depends on the *previous* tree being
right, which is exactly the assumption that makes stale accessibility state so
hard to debug.

### The threading problem

AccessKit's three handler traits are called **on a platform-dependent thread**.
A screen reader can ask for the tree at any moment, including before the first
frame. Somnium's widget tree is a pool of boxed `dyn Control` owned by the main
loop and is not `Sync`.

So the handlers get the **last published update**, behind a mutex. The main loop
publishes after the render call — because that is what ran layout, and a tree
published before it carries last frame's bounds, which puts a reader's pointer
one frame behind during exactly the interactions that move things.

`request_initial_tree` returns `None` before anything is published, which is
AccessKit's stated contract: the adapter supplies its own placeholder and
explicitly says not to return one of ours.

Actions are **queued, not handled**, because the call arrives on a platform
thread. Drained by the main loop; the test asserts draining twice does not
replay, since a click delivered twice is a click the user did not make.

### The window is now created invisible

`accesskit_winit` **panics** if its adapter is attached to a window that has
already been shown. So `Engine::resumed` builds the window with
`.with_visible(false)`, attaches the adapter, initialises the renderer and the
shell, and calls `set_visible(true)` at the end.

**This is a better startup regardless** — the window appears painted instead of
appearing and then painting — and it is verified rather than assumed: the
GHOSTFENCE golden capture still matches after the change, which means the app
still starts, still renders, and still draws the same shell.

### The gate on all of it

`is_active()` is false until a screen reader asks. For the overwhelming majority
of runs it is false for the whole session, and the accessibility path costs one
lock acquisition per frame and never walks the widget tree.

## What is not claimed

**Whether a real screen reader reads this well has not been measured.** No NVDA,
JAWS or VoiceOver session was run. §14.5 already says this sub-phase delivers no
conformance claim and this record agrees.

What *is* claimed: a correct, well-formed AccessKit tree — right roles, right
names, right nesting, right focus, every referenced child present — reaches the
platform adapter. That is the necessary condition everything else is built on,
and it is the part that can be tested without a person and a reader.

## Tests: 34 new, 0 failures

- **`somnium_ui::a11y`, 29** — an empty tree still has a root; text is a label
  and is its own name; **scaffolding is collapsed away** (asserting the node
  count, not just the survivor); a hidden subtree does not exist, with a visible
  sibling proving the test is live; a check box reports all three states; a
  slider speaks `0.44` and not an f32; an icon-only button borrows its tooltip;
  an unnamed focusable is reported and an unnamed label is not; focus follows
  the tree; announcement order and the disabled suffix; an unnamed control still
  announces its role; announcing a missing node is `None`; only `Group` is
  meaningless and only interactive roles are focusable; every role has a
  distinct spoken name; reduced motion reaches the animator and can be turned
  off again; **neither setting changes layout**; the platform query is honest;
  high contrast only ever raises the ratio, reaches 7:1, preserves polarity and
  keeps alpha; and six AccessKit conversion tests — root and focus present,
  label-into-value, disabled, mixed, bounds as a rectangle, and **every child id
  in the update exists in the update**.
- **`somnium_core::a11y_bridge`, 5** — activation before the first frame returns
  `None` not a placeholder; a published tree survives crossing to another
  thread; actions queue and drain once; deactivation drops the tree and the
  gate; an unchanged tree compares equal.

`somnium_ui`: **522 passed** (was 493). `somnium_core`: **256 passed**.

## GHOSTFENCE

```
PASS  census            MORROWIND-A_census.md matches the tree
PASS  toolchain         rustc 1.88, wgpu 30.0, winit 0.30
PASS  shader-budget     51 modules, 51 variants possible in total
PASS  one-job-system    no bare spawns; 3 exemptions, each with a reason
PASS  no-second-system  4 singleton symbols, each defined only where it is allowed
PASS  golden-images     3 image(s) within threshold
PASS  tests             1694 passed, 0 failed (floor 945)
```

**The `one-job-system` row failed first**, on the `thread::spawn` in the bridge's
cross-thread test — the gate doing exactly its job. Exempted with the reason,
which is that proving the handlers work off the main thread *is* the contract,
since the platform calls them there.

The golden row passing is load-bearing here: it is what confirms the
invisible-then-show window change did not break startup.

## Files

```
+ crates/somnium_ui/src/a11y.rs           Role, Toggled, A11yNode, A11yTree,
                                          Announcement, Politeness, A11ySettings,
                                          high_contrast, the AccessKit conversion
+ crates/somnium_ui/src/a11y/tests.rs     29 tests
+ crates/somnium_core/src/a11y_bridge.rs  the platform adapter, 5 tests
~ crates/somnium_ui/src/node.rs           four defaulted Control hooks
~ crates/somnium_ui/src/ui.rs             a11y_probe, a11y_tree, set_a11y_settings
~ crates/somnium_ui/src/lib.rs            the shell's tree and settings; re-exports
~ crates/somnium_ui/src/widgets/*.rs      twelve role overrides
~ crates/somnium_core/src/app.rs          invisible-then-show, event routing,
                                          per-frame preferences and publish
~ crates/somnium_core/src/settings.rs     reduced_motion, high_contrast
~ crates/somnium_core/src/reflect_registry.rs  their schema rows
~ tools/ghostfence/run.py                 the third spawn exemption, with its reason
```

## Track 1 (VIVEC) is closed

D (paint), E (canvas), E2 (the hook), F (input and navigation), G (text),
H (motion), I (accessibility). **One item remains open inside the track and is
recorded rather than quietly dropped: MORROWIND-G's shaper**, decided
(`cosmic-text`) and behind `SOMNIUM_UI_SHAPER`, default off. MORROWIND-E2b
removed its blocker by taking the golden reference, so the A/B is now a command
rather than an argument.
