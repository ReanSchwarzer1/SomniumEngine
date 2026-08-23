//! Phase 16-B: what the sandbox actually contains.
//!
//! These tests exist because the sandbox surface is **inherited**, not
//! authored. It is whatever the Luau version we vendor decides to put in
//! the base library, minus what we remove. A Luau upgrade can therefore
//! widen it silently, and the only defence is a test that enumerates the
//! globals and fails when the list changes.
//!
//! That is not hypothetical: `StdLib::ALL_SAFE` is `u32::MAX` under the
//! `luau` feature, so the obvious way to build a state hands scripts `os`
//! and `debug`. And even with the library flags right, the base library
//! still arrives carrying `getfenv`, `setfenv`, `loadstring`, `require`,
//! `collectgarbage`, `gcinfo` and `print`.

use somnium_script_luau::host::REMOVED_GLOBALS;
use somnium_script_luau::new_sandboxed_state;

/// Everything from [`CANDIDATE_GLOBALS`] that a script can actually see.
///
/// Probed by name rather than enumerated, because the two ways to walk an
/// environment — `getfenv(0)` and `_G` — are both things the sandbox
/// removes. A test that needed them would only pass on a VM that had the
/// hole it is testing for.
fn globals() -> Vec<String> {
    let lua = new_sandboxed_state(16 * 1024 * 1024).unwrap();
    somnium_script_luau::host::install_api(&lua).unwrap();
    lua.sandbox(true).unwrap();

    let mut names: Vec<String> = CANDIDATE_GLOBALS
        .iter()
        .filter(|name| {
            lua.load(format!("return type({name}) ~= 'nil'"))
                .eval::<bool>()
                .unwrap_or(false)
        })
        .map(|name| (*name).to_string())
        .collect();
    names.sort();
    names
}

/// Every global Luau 0.728's base library and our safe `StdLib` set can
/// produce, plus the two the engine adds.
///
/// A Luau upgrade that introduces a new global will not appear here, and
/// [`the_sandbox_surface_is_exactly_what_we_expect`] will not notice it —
/// which is why [`no_removed_global_is_reachable`] tests the dangerous
/// ones by name and why this list is reviewed on every runtime bump.
const CANDIDATE_GLOBALS: &[&str] = &[
    // Engine.
    "Script",
    "Field",
    // Base library.
    "assert",
    "error",
    "getmetatable",
    "ipairs",
    "newproxy",
    "next",
    "pairs",
    "pcall",
    "rawequal",
    "rawget",
    "rawlen",
    "rawset",
    "select",
    "setmetatable",
    "tonumber",
    "tostring",
    "type",
    "typeof",
    "unpack",
    "xpcall",
    "_VERSION",
    // Opened libraries.
    "bit32",
    "buffer",
    "coroutine",
    "math",
    "string",
    "table",
    "utf8",
    "vector",
    // Deliberately removed — must never show up.
    "getfenv",
    "setfenv",
    "loadstring",
    "require",
    "collectgarbage",
    "gcinfo",
    "print",
    "_G",
    // Never opened.
    "os",
    "debug",
    "io",
    "package",
    "dofile",
    "loadfile",
    "load",
];

#[test]
fn the_sandbox_surface_is_exactly_what_we_expect() {
    let present = globals();
    let expected: Vec<String> = [
        "Field",
        "Script",
        "_VERSION",
        "assert",
        "bit32",
        "buffer",
        "coroutine",
        "error",
        "getmetatable",
        "ipairs",
        "math",
        "newproxy",
        "next",
        "pairs",
        "pcall",
        "rawequal",
        "rawget",
        "rawlen",
        "rawset",
        "select",
        "setmetatable",
        "string",
        "table",
        "tonumber",
        "tostring",
        "type",
        "typeof",
        "unpack",
        "utf8",
        "vector",
        "xpcall",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    assert_eq!(
        present, expected,
        "the sandbox surface changed. If this is a Luau upgrade, review \
         every new global against `dev records/phase_16.md` §4.6 before \
         updating this list."
    );
}

#[test]
fn no_removed_global_is_reachable() {
    let lua = new_sandboxed_state(16 * 1024 * 1024).unwrap();
    somnium_script_luau::host::install_api(&lua).unwrap();
    lua.sandbox(true).unwrap();

    for name in REMOVED_GLOBALS {
        let present: bool = lua
            .load(format!("return type({name}) ~= 'nil'"))
            .eval()
            .unwrap();
        assert!(!present, "`{name}` must not be reachable from a script");
    }
}

#[test]
fn the_unsafe_libraries_were_never_opened() {
    let lua = new_sandboxed_state(16 * 1024 * 1024).unwrap();
    for name in ["os", "debug", "io", "package"] {
        let present: bool = lua
            .load(format!("return type({name}) ~= 'nil'"))
            .eval()
            .unwrap();
        assert!(
            !present,
            "`{name}` must never be opened — note that `StdLib::ALL_SAFE` \
             would have opened it under the luau feature"
        );
    }
}

#[test]
fn a_script_cannot_rewrite_a_shared_builtin() {
    let lua = new_sandboxed_state(16 * 1024 * 1024).unwrap();
    somnium_script_luau::host::install_api(&lua).unwrap();
    lua.sandbox(true).unwrap();

    // Sandboxing freezes the shared library tables. Without it, one script
    // redefining `math.floor` would change it for every other script in
    // the game.
    let result = lua.load("math.floor = function() return 0 end").exec();
    assert!(
        result.is_err(),
        "the shared builtins must be frozen once sandboxed"
    );
}

#[test]
fn a_memory_ceiling_is_enforced() {
    // Two megabytes, then allocate without bound. The VM must refuse
    // rather than take the process down with it.
    let lua = new_sandboxed_state(2 * 1024 * 1024).unwrap();
    let result = lua
        .load("local t = {} while true do table.insert(t, string.rep('x', 1024)) end")
        .exec();
    assert!(result.is_err(), "an allocation bomb must be stopped");
}

#[test]
fn the_engine_api_is_installed_and_frozen() {
    let lua = new_sandboxed_state(16 * 1024 * 1024).unwrap();
    somnium_script_luau::host::install_api(&lua).unwrap();
    lua.sandbox(true).unwrap();

    assert_eq!(
        lua.load("return type(Script.define)")
            .eval::<String>()
            .unwrap(),
        "function"
    );
    assert_eq!(
        lua.load("return type(Field.number)")
            .eval::<String>()
            .unwrap(),
        "function"
    );
    assert!(
        lua.load("Script.define = 1").exec().is_err(),
        "a script must not be able to replace the engine API"
    );
}
