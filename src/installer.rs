//! Запуск установки через Polkit и `pacman` без блокировки интерфейса.
//!
//! Команда запускается напрямую, без `sh -c`, поэтому путь к пакету не может
//! превратиться в shell-команду. `pkexec` показывает родной диалог KDE/GNOME,
//! а `pacman -U --noconfirm` выполняет установку уже от имени root.
//!
//! Здесь намеренно используется `std::process`, а не `GSubprocess`: дочерний
//! процесс и его потоки ввода-вывода должны свободно перемещаться в рабочий
//! поток, не затрагивая GTK/GIO-объекты главного потока.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::i18n;
use crate::WorkerMsg;

/// Итог работы дочернего процесса.
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
    /// Готовая к показу причина ошибки (если установка не удалась).
    pub error: Option<String>,
}

/// Запускает `pkexec pacman -U --noconfirm <path>`.
///
/// Функция возвращает управление сразу после создания процесса. Чтение вывода
/// и ожидание завершения происходят в отдельном потоке, поэтому GTK-цикл
/// остаётся отзывчивым, а прогресс можно анимировать.
pub fn start(path: &Path, sender: Sender<WorkerMsg>) -> Result<(), String> {
    let mut child = Command::new("pkexec")
        // Не позволяем pkexec уходить в устаревший текстовый агент: на рабочем
        // столе должен использоваться системный графический агент Polkit.
        .arg("--disable-internal-agent")
        .arg("pacman")
        .arg("-U")
        .arg("--noconfirm")
        .arg(path)
        // Рабочий каталог root-процесса не должен зависеть от каталога, из
        // которого пользователь запустил мастер (например, с флешки).
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| i18n::tr("error.install.spawn", &[("error", &error.to_string())]))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let shared_child = Arc::new(Mutex::new(child));
    let child_for_thread = Arc::clone(&shared_child);

    let thread_builder = thread::Builder::new().name("arch-swift-install".to_string());
    match thread_builder.spawn(move || {
        run_process(child_for_thread, stdout, stderr, sender);
    }) {
        Ok(_) => Ok(()),
        Err(error) => {
            // Поток не создан — завершаем уже запущенный pkexec, чтобы установка
            // не продолжилась без контроля со стороны мастера.
            if let Ok(mut child) = shared_child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            Err(i18n::tr(
                "error.install.thread",
                &[("error", &error.to_string())],
            ))
        }
    }
}

/// Читает stdout и stderr, дожидается завершения и отправляет итог в GTK-поток.
fn run_process(
    child: Arc<Mutex<Child>>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    sender: Sender<WorkerMsg>,
) {
    let stderr_sender = sender.clone();
    let stderr_reader = stderr.and_then(|stream| {
        thread::Builder::new()
            .name("arch-swift-pacman-stderr".to_string())
            .spawn(move || forward_output(stream, stderr_sender))
            .ok()
    });

    let stdout_error = stdout.and_then(|stream| forward_output(stream, sender.clone()));
    let stderr_error = stderr_reader
        .and_then(|handle| handle.join().ok())
        .flatten();
    let read_error = stdout_error.or(stderr_error);

    let status = match child.lock() {
        Ok(mut process) => process.wait(),
        Err(_) => {
            let _ = sender.send(WorkerMsg::Finished(InstallOutcome {
                success: false,
                exit_code: None,
                error: Some(i18n::tr(
                    "error.install.wait",
                    &[("error", "mutex poisoned")],
                )),
            }));
            return;
        }
    };

    let status = match status {
        Ok(status) => status,
        Err(error) => {
            let _ = sender.send(WorkerMsg::Finished(InstallOutcome {
                success: false,
                exit_code: None,
                error: Some(i18n::tr(
                    "error.install.wait",
                    &[("error", &error.to_string())],
                )),
            }));
            return;
        }
    };

    let success = status.success();
    let exit_code = status.code();

    let error = if success {
        None
    } else {
        read_error.or_else(|| failure_message(exit_code))
    };

    if sender
        .send(WorkerMsg::Finished(InstallOutcome {
            success,
            exit_code,
            error,
        }))
        .is_err()
    {
        log_info!("The installation result was not delivered: the window is already closed.");
    }
}

/// Построчно пересылает вывод процесса, корректно обрабатывая не-UTF-8 байты.
/// Возвращает локализованную ошибку чтения, если поток оборвался.
fn forward_output<R: Read>(stream: R, sender: Sender<WorkerMsg>) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => return None,
            Ok(_) => {
                let text = String::from_utf8_lossy(&buffer);
                let line = text.trim_end_matches(['\r', '\n']).to_string();
                if sender.send(WorkerMsg::Log(line)).is_err() {
                    // Окно уже закрыто — дальше передавать вывод некому.
                    return None;
                }
            }
            Err(error) => {
                let message = i18n::tr("error.install.read", &[("error", &error.to_string())]);
                let _ = sender.send(WorkerMsg::Log(message.clone()));
                return Some(message);
            }
        }
    }
}

/// Переводит коды выхода `pkexec`/`pacman` в понятные пользователю сообщения.
fn failure_message(exit_code: Option<i32>) -> Option<String> {
    let message = match exit_code {
        Some(126) => i18n::tr("error.install.auth_cancelled", &[]),
        Some(127) => i18n::tr("error.install.not_authorized", &[]),
        Some(125) => i18n::tr("error.install.pkexec_failed", &[]),
        Some(code) => i18n::tr("error.install.pacman_code", &[("code", &code.to_string())]),
        None => i18n::tr("error.install.signaled", &[]),
    };

    Some(message)
}
