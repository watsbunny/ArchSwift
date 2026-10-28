//! Сборка встроенных ресурсов и реестра переводов.
//!
//! Здесь решаются две задачи, которые должны происходить до компиляции:
//!   1. `glib-compile-resources` упаковывает SVG-иконки из `assets/` в
//!      GResource — приложение не зависит от установленной темы hicolor;
//!   2. каталоги `localization/lang/*.toml` превращаются в срез `CATALOGS`,
//!      поэтому новый язык достаточно добавить файлом, не трогая Rust-код.
//!
//! Оба генератора сообщают Cargo, какие файлы отслеживать, поэтому правка
//! иконки или перевода автоматически запускает повторную сборку.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    compile_resources();
    compile_translations();
}

/// Упаковывает иконки в GResource, который регистрируется в `main.rs`.
fn compile_resources() {
    glib_build_tools::compile_resources(
        &["assets"],
        "assets/resources.gresource.xml",
        "arch-swift.gresource",
    );
}

/// Генерирует `languages.rs` со списком каталогов переводов.
///
/// Английский каталог обязателен: он используется как fallback, если ключ
/// отсутствует в выбранном языке или сам файл перевода повреждён.
fn compile_translations() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let language_dir = manifest_dir.join("localization").join("lang");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    // Каталог отслеживается целиком: так Cargo заметит и новый файл языка.
    println!("cargo:rerun-if-changed={}", language_dir.display());

    let mut catalogs = discover_catalogs(&language_dir);

    assert!(
        catalogs.iter().any(|(code, _)| code == "en"),
        "localization/lang/en.toml is required: it is the fallback catalog"
    );

    // Стабильный порядок делает сборку воспроизводимой.
    catalogs.sort_by(|left, right| left.0.cmp(&right.0));

    let mut generated = String::new();
    generated.push_str("/// Каталоги переводов, найденные в localization/lang во время сборки.\n");
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

/// Собирает пары «код языка — путь к TOML» из каталога переводов.
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

        // Код языка попадает в исходный код Rust, поэтому допускаем только
        // безопасные символы: en, ru, be, pt-BR, zh_Hans и т.п.
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
