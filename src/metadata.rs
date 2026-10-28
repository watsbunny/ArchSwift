//! Разбор локальных пакетов Arch Linux без их установки.
//!
//! Пакет Arch Linux — это обычный tar-архив, сжатый zstd (современный формат),
//! xz, gzip либо несжатый. Сразу после заголовков makepkg кладёт служебный файл
//! `.PKGINFO`, поэтому для отображения сведений не нужно распаковывать весь пакет:
//! чтение прекращается, как только начинается область полезной нагрузки.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use gtk::glib;

use crate::i18n;

/// Метаданные, необходимые мастеру для первого экрана.
#[derive(Debug, Clone, Default)]
pub struct PackageInfo {
    /// Канонизированный абсолютный путь к файлу пакета.
    pub path: PathBuf,
    /// Размер сжатого файла на диске.
    pub file_size: u64,
    /// Найден ли рядом файл подписи `<package>.sig`.
    pub signed: bool,
    pub pkgname: String,
    pub pkgver: String,
    pub pkgdesc: String,
    pub url: String,
    pub packager: String,
    pub arch: String,
    /// Unix-время сборки пакета.
    pub builddate: Option<i64>,
    /// Размер после установки, как его указывает makepkg в `.PKGINFO`.
    pub installed_size: Option<u64>,
    pub licenses: Vec<String>,
    pub depends: Vec<String>,
    pub optdepends: Vec<String>,
    pub provides: Vec<String>,
    pub conflicts: Vec<String>,
    pub replaces: Vec<String>,
    pub groups: Vec<String>,
    /// Наличие `.INSTALL` означает, что пакет выполняет собственные скрипты.
    pub has_install_script: bool,
}

impl PackageInfo {
    /// Короткое имя вида `pkgname pkgver` для заголовков и журналов.
    pub fn summary(&self) -> String {
        format!("{} {}", self.pkgname, self.pkgver)
            .trim()
            .to_string()
    }

    /// Дата сборки в нейтральном формате `ГГГГ-ММ-ДД ЧЧ:ММ`.
    pub fn build_date(&self) -> Option<String> {
        let timestamp = self.builddate?;

        glib::DateTime::from_unix_local(timestamp)
            .ok()
            .and_then(|date_time| date_time.format("%Y-%m-%d %H:%M").ok())
            .map(|formatted| formatted.to_string())
    }
}

/// Читает и проверяет пакет. Функция не требует root-прав и не изменяет систему.
pub fn load(path: &Path) -> Result<PackageInfo, String> {
    let canonical = path.canonicalize().map_err(|error| {
        localized_error(
            "error.package.open",
            &[
                ("path", &path.display().to_string()),
                ("error", &error.to_string()),
            ],
        )
    })?;

    let file_metadata = std::fs::metadata(&canonical).map_err(|error| {
        localized_error(
            "error.package.stat",
            &[
                ("path", &canonical.display().to_string()),
                ("error", &error.to_string()),
            ],
        )
    })?;

    if !file_metadata.is_file() {
        return Err(localized_error(
            "error.package.not_file",
            &[("path", &canonical.display().to_string())],
        ));
    }

    let file_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    if !file_name.contains(".pkg.tar") {
        return Err(localized_error(
            "error.package.bad_name",
            &[("path", &canonical.display().to_string())],
        ));
    }

    let reader = open_reader(&canonical)?;
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|error| {
        localized_error("error.package.tar_open", &[("error", &error.to_string())])
    })?;

    let mut pkginfo_text: Option<String> = None;
    let mut has_install_script = false;
    let mut payload_started = false;

    for entry in entries {
        let mut entry = entry.map_err(|error| {
            localized_error("error.package.tar_entry", &[("error", &error.to_string())])
        })?;

        let entry_path = entry.path().map_err(|error| {
            localized_error("error.package.entry_path", &[("error", &error.to_string())])
        })?;
        let entry_name = normalize_entry_path(&entry_path);

        if entry_name == ".PKGINFO" {
            let mut text = String::new();
            entry.read_to_string(&mut text).map_err(|error| {
                localized_error(
                    "error.package.pkginfo_read",
                    &[("error", &error.to_string())],
                )
            })?;
            pkginfo_text = Some(text);

            if has_install_script {
                break;
            }
        } else if entry_name == ".INSTALL" {
            has_install_script = true;

            if pkginfo_text.is_some() {
                break;
            }
        } else if !entry_name.starts_with('.') {
            // Стандартный makepkg кладёт все служебные файлы до полезной нагрузки.
            // Дальше читать архив нет смысла — это только замедлило бы мастер.
            payload_started = true;

            if pkginfo_text.is_some() {
                break;
            }
        }
    }

    if payload_started && pkginfo_text.is_none() {
        return Err(i18n::tr("error.package.pkginfo_order", &[]));
    }

    let pkginfo_text =
        pkginfo_text.ok_or_else(|| i18n::tr("error.package.pkginfo_missing", &[]))?;

    let mut info = PackageInfo {
        path: canonical.clone(),
        file_size: file_metadata.len(),
        signed: signature_exists(&canonical),
        has_install_script,
        ..PackageInfo::default()
    };

    parse_pkginfo(&mut info, &pkginfo_text)?;

    Ok(info)
}

/// Собирает локализованное сообщение об ошибке из шаблона и аргументов.
fn localized_error(key: &str, args: &[(&str, &str)]) -> String {
    i18n::tr(key, args)
}

/// Открывает поток чтения с учётом фактического расширения пакета.
fn open_reader(path: &Path) -> Result<Box<dyn Read>, String> {
    let file = File::open(path).map_err(|error| {
        localized_error(
            "error.package.open",
            &[
                ("path", &path.display().to_string()),
                ("error", &error.to_string()),
            ],
        )
    })?;
    let buffered = BufReader::new(file);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    if name.ends_with(".zst") {
        let decoder = zstd::stream::read::Decoder::new(buffered).map_err(|error| {
            localized_error("error.package.zstd", &[("error", &error.to_string())])
        })?;
        Ok(Box::new(decoder))
    } else if name.ends_with(".xz") {
        Ok(Box::new(xz2::read::XzDecoder::new(buffered)))
    } else if name.ends_with(".gz") {
        Ok(Box::new(flate2::read::GzDecoder::new(buffered)))
    } else if name.ends_with(".bz2") || name.ends_with(".lrz") || name.ends_with(".lzo") {
        Err(localized_error(
            "error.package.unsupported",
            &[("path", &path.display().to_string())],
        ))
    } else if name.ends_with(".pkg.tar") {
        Ok(Box::new(buffered))
    } else {
        Err(localized_error(
            "error.package.unknown_format",
            &[("path", &path.display().to_string())],
        ))
    }
}

/// Приводит путь элемента tar к виду без ведущего `./`.
fn normalize_entry_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

/// Проверяет наличие отдельного файла подписи, не изменяя исходный путь.
fn signature_exists(path: &Path) -> bool {
    let mut signature = path.as_os_str().to_os_string();
    signature.push(".sig");
    PathBuf::from(signature).is_file()
}

/// Разбирает формат `.PKGINFO` (`ключ = значение`, повторяющиеся ключи).
fn parse_pkginfo(info: &mut PackageInfo, text: &str) -> Result<(), String> {
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }

        match key {
            "pkgname" => info.pkgname = value.to_string(),
            "pkgver" => info.pkgver = value.to_string(),
            "pkgdesc" => info.pkgdesc = value.to_string(),
            "url" => info.url = value.to_string(),
            "packager" => info.packager = value.to_string(),
            "arch" => info.arch = value.to_string(),
            "builddate" => info.builddate = value.parse::<i64>().ok(),
            "size" => info.installed_size = value.parse::<u64>().ok(),
            "license" => info.licenses.push(value.to_string()),
            "depend" => info.depends.push(value.to_string()),
            "optdepend" => info.optdepends.push(value.to_string()),
            "provides" => info.provides.push(value.to_string()),
            "conflicts" => info.conflicts.push(value.to_string()),
            "replaces" => info.replaces.push(value.to_string()),
            "group" => info.groups.push(value.to_string()),
            // Остальные поля (.BUILDINFO-совместимые, xdata и т.п.) мастеру не нужны.
            _ => {}
        }
    }

    if info.pkgname.is_empty() {
        return Err(i18n::tr("error.pkginfo.pkgname", &[]));
    }
    if info.pkgver.is_empty() {
        return Err(i18n::tr("error.pkginfo.pkgver", &[]));
    }

    Ok(())
}

/// Форматирует размер в двоичных единицах.
///
/// Обозначения B/KiB/MiB не переводятся: это общепринятые технические
/// сокращения, одинаково понятные во всех трёх локалях.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
pkgname = demo-package
pkgver = 1.2.3-1
pkgdesc = Демонстрационный пакет
url = https://example.org
arch = x86_64
builddate = 1700000000
packager = Test Builder <test@example.org>
size = 2048
license = MIT
depend = glibc
depend = gtk4>=4.10
optdepend = python: для дополнительных сценариев
provides = demo=1.2.3
conflicts = demo-old
group = demo
";

    #[test]
    fn parses_repeated_and_single_fields() {
        let mut info = PackageInfo::default();
        parse_pkginfo(&mut info, SAMPLE).expect("образец должен разбираться");

        assert_eq!(info.pkgname, "demo-package");
        assert_eq!(info.pkgver, "1.2.3-1");
        assert_eq!(info.arch, "x86_64");
        assert_eq!(info.installed_size, Some(2048));
        assert_eq!(info.depends, vec!["glibc", "gtk4>=4.10"]);
        assert_eq!(info.optdepends.len(), 1);
        assert_eq!(info.provides, vec!["demo=1.2.3"]);
        assert_eq!(info.conflicts, vec!["demo-old"]);
        assert_eq!(info.groups, vec!["demo"]);
    }

    #[test]
    fn rejects_missing_required_fields() {
        let mut info = PackageInfo::default();
        let error = parse_pkginfo(&mut info, "pkgver = 1.0-1").unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn formats_human_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KiB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn normalizes_tar_paths() {
        assert_eq!(normalize_entry_path(Path::new("./.PKGINFO")), ".PKGINFO");
        assert_eq!(normalize_entry_path(Path::new(".INSTALL")), ".INSTALL");
    }
}
