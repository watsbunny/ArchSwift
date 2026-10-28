//! Building embedded resources and the translation registry.
//!
//! Two tasks must happen before compilation:
//!   1. `glib-compile-resources` packs the SVG icons from `assets/` into a
//!      GResource — the application does not depend on the installed hicolor theme;
//!   2. the `localization/lang/*.toml` catalogs become the `CATALOGS` slice,
//!      so adding a language is just adding a file, with no Rust changes.
//!
//! Both generators tell Cargo which files to watch, so editing an icon or a
//! translation automatically triggers a rebuild.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    compile_resources();
    compile_translations();
}

/// Packs the icons into a GResource that is registered in `main.rs`.
fn compile_resources() {
    glib_build_tools::compile_resources(
        &["assets"],
        "assets/resources.gresource.xml",
        "arch-swift.gresource",
    );
}

/// Generates `languages.rs` with the list of translation catalogs.
///
/// The English catalog is mandatory: it is the fallback when a key is missing
/// in the selected language or when a translation file itself is corrupted.
fn compile_translations() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let language_dir = manifest_dir.join("localization").join("lang");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    // The whole directory is watched: that way Cargo notices new language files.
    println!("cargo:rerun-if-changed={}", language_dir.display());

    let mut catalogs = discover_catalogs(&language_dir);

    assert!(
        catalogs.iter().any(|(code, _)| code == "en"),
        "localization/lang/en.toml is required: it is the fallback catalog"
    );

    // A stable order makes the build reproducible.
    catalogs.sort_by(|left, right| left.0.cmp(&right.0));

    let mut generated = String::new();
    generated.push_str("/// Translation catalogs discovered in localization/lang at build time.\n");
    generated.push_str("pub const CATALOGS: &[(&str, &str)] = &[\n");

    for (code, path) in catalogs {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("translation file name must be valid UTF-8");

        println!("cargo:rerun-if-changed={}", path.display());
        generated.push_str(&format!(
            "    (\"{code}\", include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \
             \"/localization/lang/{file_name}\"))),\n"
        ));
    }

    generated.push_str("];\n");

    fs::write(out_dir.join("languages.rs"), generated)
        .expect("failed to write the generated language registry");
}

/// Collects "language code — TOML path" pairs from the translation directory.
fn discover_catalogs(directory: &Path) -> Vec<(String, PathBuf)> {
    let entries = fs::read_dir(directory).unwrap_or_else(|error| {
        panic!(
            "cannot read translation directory {}: {error}",
            directory.display()
        )
    });

    let mut catalogs = Vec::new();
    for entry in entries {
        let path = entry.expect("cannot read a translation entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }

        let code = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_else(|| panic!("translation file {} has no valid name", path.display()))
            .to_string();

        // The language code ends up in Rust source, so only safe characters are
        // allowed: en, ru, be, pt-BR, zh_Hans and similar.
        assert!(
            !code.is_empty()
                && code
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()
                        || character == '-'
                        || character == '_'),
            "invalid language code in file name: {}",
            path.display()
        );

        catalogs.push((code, path));
    }

    catalogs
}
