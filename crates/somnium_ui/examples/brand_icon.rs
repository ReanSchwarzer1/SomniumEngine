//! Write the engine mark out as a Windows `.ico`.
//!
//! ```text
//! cargo run -p somnium_ui --example brand_icon -- crates/somnium_ui/assets/brand/somnium.ico
//! ```
//!
//! The file is committed rather than built, because the alternative is a build
//! script that rasterises SVG on every machine that compiles the editor, for a
//! drawing that changes about once a year. This is how it is regenerated when
//! that year comes round, and `PROVENANCE.md` says so beside the sources.

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: brand_icon <out.ico>");
        std::process::exit(2);
    });
    let Some(bytes) = somnium_ui::brand_ico::engine_mark(somnium_ui::theme::ACCENT) else {
        eprintln!("the vendored brand drawing did not rasterise");
        std::process::exit(1);
    };
    match std::fs::write(&path, &bytes) {
        Ok(()) => println!(
            "wrote {path}: {} bytes, sizes {:?}",
            bytes.len(),
            somnium_ui::brand_ico::SIZES
        ),
        Err(error) => {
            eprintln!("could not write {path}: {error}");
            std::process::exit(1);
        }
    }
}
