//! Лёгкая система локализации ArchSwift.
//!
//! Переводы лежат в `localization/lang/<lang>.toml`. `build.rs` во время
//! сборки сканирует этот каталог и генерирует реестр `CATALOGS`, а содержимое
//! файлов встраивается в бинарник через `include_str!`. В результате:
//!   - новый язык добавляется одним TOML-файлом, без правок Rust-кода;
//!   - приложению не нужно искать переводы в файловой системе.
//!
//! Язык выбирается по переменным окружения в порядке приоритета:
//! `ARCHSWIFT_LANG`, `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`.
//! Если ни одна переменная не дала поддерживаемого языка, используется
//! английский. Неизвестный `ARCHSWIFT_LANG` тоже даёт английский: это явное
//! переопределение, и оно не должно зависеть от системной локали.

use std::collections::HashMap;
use std::sync::OnceLock;

// Реестр генерируется build.rs: &[(код языка, содержимое TOML)].
include!(concat!(env!("OUT_DIR"), "/languages.rs"));

/// Язык, который используется как запасной для отсутствующих ключей.
const FALLBACK_LANGUAGE: &str = "en";

struct Catalog {
    language: &'static str,
    messages: HashMap<String, String>,
    fallback: HashMap<String, String>,
}

impl Catalog {
    fn lookup<'a>(&'a self, key: &'a str) -> &'a str {
        self.messages
            .get(key)
            .or_else(|| self.fallback.get(key))
            .map(String::as_str)
            .unwrap_or(key)
    }
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

/// Код активного языка (`en`, `ru`, `be` или другой из `localization/lang`).
pub fn language() -> &'static str {
    catalog().language
}

/// Возвращает перевод по ключу и подставляет именованные аргументы `{name}`.
///
/// Если ключ отсутствует в выбранном языке, используется английский вариант.
/// Если ключа нет и там, возвращается сам ключ — это удобно замечать при
/// разработке и не приводит к падению приложения.
pub fn tr(key: &str, args: &[(&str, &str)]) -> String {
    interpolate(catalog().lookup(key), args)
}

fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| {
        let fallback = parse_catalog(source(FALLBACK_LANGUAGE)).unwrap_or_else(|error| {
            eprintln!(
                "[ArchSwift] Failed to parse {FALLBACK_LANGUAGE}.toml: {error}. \
                 The interface will fall back to translation keys."
            );
            HashMap::new()
        });

        let language = detect_language();
        let messages = if language == FALLBACK_LANGUAGE {
            fallback.clone()
        } else {
            parse_catalog(source(language)).unwrap_or_else(|error| {
                eprintln!(
                    "[ArchSwift] Failed to parse {language}.toml: {error}. \
                     Falling back to English."
                );
                fallback.clone()
            })
        };

        Catalog {
            language,
            messages,
            fallback,
        }
    })
}

/// Возвращает встроенный каталог по коду языка.
fn source(language: &str) -> &'static str {
    catalogs()
        .iter()
        .find(|(code, _)| *code == language)
        .map(|(_, source)| *source)
        // build.rs гарантирует наличие английского каталога.
        .unwrap_or_else(|| {
            catalogs()
                .iter()
                .find(|(code, _)| *code == FALLBACK_LANGUAGE)
                .map(|(_, source)| *source)
                .expect("the English catalog must be embedded")
        })
}

fn catalogs() -> &'static [(&'static str, &'static str)] {
    CATALOGS
}

/// Разбирает TOML и разворачивает секции в плоские ключи `section.key`.
fn parse_catalog(source: &str) -> Result<HashMap<String, String>, toml::de::Error> {
    let table: toml::Table = toml::from_str(source)?;
    let mut messages = HashMap::new();
    flatten_table("", &table, &mut messages);
    Ok(messages)
}

fn flatten_table(prefix: &str, table: &toml::Table, output: &mut HashMap<String, String>) {
    for (key, value) in table {
        let full_key = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        match value {
            toml::Value::String(text) => {
                output.insert(full_key, text.clone());
            }
            toml::Value::Table(nested) => flatten_table(&full_key, nested, output),
            _ => {
                // Каталог переводов содержит только строки; прочие значения
                // игнорируются, чтобы не ломать загрузку из-за опечатки.
            }
        }
    }
}

/// Определяет язык интерфейса по окружению.
fn detect_language() -> &'static str {
    // Явное переопределение: если ARCHSWIFT_LANG задан, системные локали уже не
    // учитываются. Неизвестный код при этом даёт английский, а не случайный
    // язык из LC_ALL — так тестирование остаётся предсказуемым.
    if let Ok(value) = std::env::var("ARCHSWIFT_LANG") {
        if !value.trim().is_empty() {
            return match_locale(&value).unwrap_or(FALLBACK_LANGUAGE);
        }
    }

    // LANGUAGE — список предпочтений через двоеточие (GNU gettext).
    if let Ok(value) = std::env::var("LANGUAGE") {
        for locale in value.split(':') {
            if let Some(language) = match_locale(locale) {
                return language;
            }
        }
    }

    for variable in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(variable) {
            if !value.is_empty() {
                if let Some(language) = match_locale(&value) {
                    return language;
                }
            }
        }
    }

    FALLBACK_LANGUAGE
}

/// Приводит локаль вида `ru_RU.UTF-8` к коду встроенного каталога.
///
/// Сначала ищется точное совпадение с кодом файла (например, `pt-BR`), затем
/// базовый язык без региона (`pt`), поэтому региональные каталоги работают
/// как специализация общего перевода.
fn match_locale(locale: &str) -> Option<&'static str> {
    let normalized = locale.trim().to_ascii_lowercase();
    let without_encoding = normalized.split(['.', '@']).next().unwrap_or_default();
    let tag = without_encoding.replace('_', "-");
    let base = tag.split('-').next().unwrap_or_default();

    catalogs()
        .iter()
        .map(|(code, _)| *code)
        .find(|code| code.eq_ignore_ascii_case(&tag))
        .or_else(|| {
            catalogs()
                .iter()
                .map(|(code, _)| *code)
                .find(|code| code.eq_ignore_ascii_case(base))
        })
}

fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    if args.is_empty() {
        return template.to_string();
    }

    let mut result = template.to_string();
    for (name, value) in args {
        result = result.replace(&format!("{{{name}}}"), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_catalog_is_embedded() {
        assert!(
            catalogs()
                .iter()
                .any(|(code, _)| *code == FALLBACK_LANGUAGE),
            "build.rs должен встроить localization/lang/en.toml"
        );
    }

    #[test]
    fn all_catalogs_have_the_same_keys() {
        let english =
            parse_catalog(source(FALLBACK_LANGUAGE)).expect("en.toml должен быть корректным TOML");

        for (language, catalog_source) in catalogs() {
            if *language == FALLBACK_LANGUAGE {
                continue;
            }

            let translated = parse_catalog(catalog_source)
                .unwrap_or_else(|_| panic!("{language}.toml повреждён"));

            let mut missing: Vec<&String> = english
                .keys()
                .filter(|key| !translated.contains_key(*key))
                .collect();
            missing.sort();

            let mut extra: Vec<&String> = translated
                .keys()
                .filter(|key| !english.contains_key(*key))
                .collect();
            extra.sort();

            assert!(
                missing.is_empty(),
                "{language}.toml: отсутствуют ключи {missing:?}"
            );
            assert!(
                extra.is_empty(),
                "{language}.toml: лишние ключи без английского оригинала {extra:?}"
            );
        }
    }

    #[test]
    fn interpolation_replaces_named_arguments() {
        assert_eq!(
            interpolate("Пакет {package} готов", &[("package", "demo-1.0-1")]),
            "Пакет demo-1.0-1 готов"
        );
    }

    #[test]
    fn locale_matching_handles_encoding_and_territory() {
        assert_eq!(match_locale("ru_RU.UTF-8"), Some("ru"));
        assert_eq!(match_locale("be_BY.UTF-8"), Some("be"));
        assert_eq!(match_locale("en_US"), Some("en"));
        assert_eq!(match_locale("C"), None);
    }
}
