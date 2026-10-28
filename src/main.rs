//! ArchSwift — графический мастер установки локальных пакетов Arch Linux.
//!
//! Программа рассчитана на запуск двойным щелчком по файлу `*.pkg.tar.zst`.
//! Архитектура специально разделена на три слоя:
//!   1. `metadata`  — безопасное чтение `.PKGINFO` из сжатого tar-архива;
//!   2. `installer` — запуск `pkexec pacman -U` в отдельном потоке без блокировки GTK;
//!   3. `ui`        — трёхшаговый мастер на GTK4 с обменом сообщениями через канал.
//!
//! Все обращения к файловой системе и внешним процессам возвращают понятные
//! локализованные ошибки. Пользовательские данные никогда не подставляются
//! в shell: `pkexec` запускается напрямую через `std::process::Command`
//! с массивом аргументов.

use std::path::PathBuf;

use gtk::glib;
use gtk::prelude::*;
use gtk::{gio, Application};

/// Единая точка журналирования. Префикс облегчает поиск в journalctl/stdout.
/// Макрос объявлен до модулей, чтобы `log_info!` был виден во всех дочерних модулях.
macro_rules! log_info {
    ($($arg:tt)*) => {
        eprintln!("[{}] {}", crate::APP_NAME, format_args!($($arg)*))
    };
}

mod i18n;
mod installer;
mod metadata;
mod ui;

/// Идентификатор приложения в терминах freedesktop.org / Wayland / D-Bus.
pub const APP_ID: &str = "org.openarch.swift";
/// Человекочитаемое имя, используемое в заголовках окон и журналах.
pub const APP_NAME: &str = "ArchSwift";
/// Имя иконки самого приложения из темы hicolor.
pub const ICON_APP: &str = "arch-swift-installer";

/// Сообщения, которые фоновые потоки передают в GTK-поток.
///
/// Канал намеренно не содержит GTK-типов: это позволяет работать с ним
/// из обычных `std::thread` и не нарушать правила потокобезопасности GTK.
enum WorkerMsg {
    /// Результат разбора метаданных пакета. Box уменьшает размер сообщения канала.
    Metadata(Result<Box<metadata::PackageInfo>, String>),
    /// Одна строка вывода `pkexec`/`pacman`.
    Log(String),
    /// Итог установки.
    Finished(installer::InstallOutcome),
}

fn main() -> glib::ExitCode {
    // Иконки упакованы в GResource на этапе сборки, поэтому мастер выглядит
    // одинаково и до установки пакета, и после неё.
    gio::resources_register_include!("arch-swift.gresource")
        .expect("не удалось зарегистрировать встроенные ресурсы");

    // Аргумент `%f` из .desktop-файла: путь к пакету, открытому двойным щелчком.
    // Если программа запущена вручную без аргумента, мастер предложит выбрать файл.
    let initial_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());

    // NON_UNIQUE позволяет открыть несколько мастеров одновременно — по одному
    // на каждый пакет. Это ближе к привычному поведению установщиков Windows.
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(move |application| {
        log_info!(
            "Starting {} (application-id: {}, language: {})",
            APP_NAME,
            APP_ID,
            i18n::language()
        );
        ui::build(application, initial_path.clone());
    });

    // GApplication не должен разбирать путь к пакету как файловый аргумент:
    // без флага HANDLES_OPEN это приводит к критическому сообщению
    // «This application can not open files» и завершению с ошибкой.
    // Путь уже прочитан вручную выше, поэтому GTK получает только имя программы.
    let program = std::env::args()
        .next()
        .unwrap_or_else(|| APP_NAME.to_string());

    app.run_with_args(&[program])
}
