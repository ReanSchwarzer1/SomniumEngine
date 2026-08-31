//! A shader error names the file it is in (DREAMS-A).
//!
//! Composition concatenates up to eight modules into one string and naga
//! reports against that string, so the raw diagnostic for `shading.wgsl` is a
//! line number in 209 KB of text with no file name. Measured before this
//! landed: a mistake on line 48 of the 120-line `brdf.wgsl` arrived as
//! `wgsl:195`, and the renderer prefixed it with `shading.wgsl`, which is a
//! file the error is not in.
//!
//! This test is the regression, and it is in `somnium_renderer` rather than in
//! `somnium_shader` because naga is what produces the line number, and
//! `somnium_shader` deliberately depends on `wgpu` and nothing else.

use somnium_shader::{Defines, ShaderKey, ShaderSystem};

/// Every module the renderer registers, in `shaders.rs` order.
///
/// Duplicated from `shaders.rs` on purpose: that list is built by a macro over
/// `include_str!`, and a test that read the same macro would pass if the macro
/// were the thing that broke.
const MODULES: &[&str] = &[
    "atmosphere.wgsl",
    "brdf.wgsl",
    "clipmap_shade.wgsl",
    "global_pool.wgsl",
    "hextile.wgsl",
    "sampling.wgsl",
    "shading.wgsl",
    "terrain_material.wgsl",
];

fn shader_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders")
}

/// Register the shading pass's eight modules, with one line of one of them
/// replaced.
fn system_with(broken: Option<(&str, usize, &str)>) -> ShaderSystem {
    let mut system = ShaderSystem::new();
    system.register_define(0, "SKINNED");
    for name in MODULES {
        let text = std::fs::read_to_string(shader_dir().join(name))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let text = match broken {
            Some((target, line, replacement)) if target == *name => text
                .lines()
                .enumerate()
                .map(|(index, original)| {
                    if index + 1 == line {
                        replacement.to_string()
                    } else {
                        original.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => text,
        };
        system.register(name, text);
    }
    system
}

fn shading_key(system: &ShaderSystem) -> ShaderKey {
    ShaderKey {
        module: system
            .registry()
            .id("shading.wgsl")
            .expect("shading.wgsl is registered"),
        defines: Defines::NONE,
    }
}

#[test]
fn an_error_in_an_included_module_is_reported_against_that_module() {
    // A statement line in the middle of `brdf.wgsl`, chosen from the file
    // rather than hard-coded so the test survives the file being edited.
    let brdf = std::fs::read_to_string(shader_dir().join("brdf.wgsl")).expect("brdf.wgsl");
    let (index, original) = brdf
        .lines()
        .enumerate()
        .find(|(_, line)| line.trim_start().starts_with("let "))
        .expect("brdf.wgsl has a `let`");
    let broken_line = index + 1;

    let mut system = system_with(Some((
        "brdf.wgsl",
        broken_line,
        // Valid on its own line nowhere: an `@` where an identifier belongs.
        "    let @ = 1.0;",
    )));
    let key = shading_key(&system);
    let source = system
        .source(key)
        .expect("a syntax error still composes; composition is textual")
        .to_string();

    let error = naga::front::wgsl::parse_str(&source)
        .err()
        .expect("the injected `@` is not valid WGSL");
    let location = error
        .location(&source)
        .expect("a parse error carries a span");

    let origin = system
        .locate(key, location.line_number as usize)
        .expect("the failing line came from a module");

    assert_eq!(
        origin.module, "brdf.wgsl",
        "the diagnostic named the wrong file; it used to name the root, \
         `shading.wgsl`, which is a file the error is not in"
    );
    assert_eq!(
        origin.line,
        broken_line,
        "wrong line in the right file (original was `{}`)",
        original.trim()
    );
    assert_ne!(
        location.line_number as usize, broken_line,
        "the composed line and the source line happened to coincide, so this \
         run proves nothing; break a line further into the file"
    );
}

#[test]
fn the_map_covers_every_line_of_a_real_composed_shader() {
    // A hole in the map is a diagnostic that silently falls back to naming the
    // root, which is the behaviour this replaced.
    let mut system = system_with(None);
    let key = shading_key(&system);
    let source = system.source(key).expect("composes").to_string();
    let lines = source.lines().count();
    assert!(
        lines > 1_000,
        "expected the whole shading pass, got {lines}"
    );

    // Line 1 is the `enable` hoisted out of `global_pool.wgsl`, which belongs
    // to no single module by design.
    let header = usize::from(source.starts_with("enable "));
    let unmapped: Vec<usize> = (1 + header..=lines)
        .filter(|line| system.locate(key, *line).is_none())
        .collect();
    assert!(
        unmapped.is_empty(),
        "{} composed lines map to no module, first at {:?}",
        unmapped.len(),
        unmapped.first()
    );
}

#[test]
fn a_located_line_holds_the_text_the_composed_line_holds() {
    // The strongest check available without a GPU: for a sample of composed
    // lines, the module and line the map names must contain the same text.
    let mut system = system_with(None);
    let key = shading_key(&system);
    let source = system.source(key).expect("composes").to_string();
    let composed: Vec<&str> = source.lines().collect();

    let mut checked = 0usize;
    for (index, text) in composed.iter().enumerate() {
        let line = index + 1;
        if line % 37 != 0 {
            continue;
        }
        let Some(origin) = system.locate(key, line) else {
            continue;
        };
        let module = std::fs::read_to_string(shader_dir().join(origin.module))
            .unwrap_or_else(|error| panic!("{}: {error}", origin.module));
        let actual = module
            .lines()
            .nth(origin.line - 1)
            .unwrap_or_else(|| panic!("{}:{} is past the end", origin.module, origin.line));
        assert_eq!(
            actual, *text,
            "composed line {line} says it is {}:{}, but that line reads differently",
            origin.module, origin.line
        );
        checked += 1;
    }
    assert!(checked > 20, "only {checked} lines sampled");
}
