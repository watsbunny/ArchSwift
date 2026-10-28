//! Running the installation through Polkit and `pacman` without blocking the UI.
//!
//! The command is executed directly, without `sh -c`, so the package path can
//! never turn into a shell command. `pkexec` shows the native KDE/GNOME dialog,
//! and `pacman -U --noconfirm` performs the installation as root.
//!
//! `std::process` is used here deliberately instead of `GSubprocess`: the child
//! process and its I/O streams must be free to move to a worker thread without
//! touching GTK/GIO objects of the main thread.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::i18n;
use crate::WorkerMsg;

/// Result of the child process.
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
    /// A ready-to-show error reason (when the installation failed).
    pub error: Option<String>,
}

/// Runs `pkexec pacman -U --noconfirm <path>`.
///
/// The function returns right after spawning the process. Output reading and
/// waiting for completion happen in a separate thread, so the GTK loop stays
/// responsive and the progress can be animated.
pub fn start(path: &Path, sender: Sender<WorkerMsg>) -> Result<(), String> {
    let mut child = Command::new("pkexec")
        // Do not let pkexec fall back to the legacy text agent: on a desktop the
        // system graphical Polkit agent must be used.
        .arg("--disable-internal-agent")
        .arg("pacman")
        .arg("-U")
        .arg("--noconfirm")
        .arg(path)
        // The root process working directory must not depend on the directory the user
        // started the wizard from (for example, a USB stick).
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
            // The thread was not created — terminate the already started pkexec so the
            // installation does not continue without the wizard being in control.
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

/// Reads stdout and stderr, waits for completion and sends the result to the GTK thread.
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

/// Forwards the process output line by line, handling non-UTF-8 bytes correctly.
/// Returns a localized read error if the stream broke.
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
                    // The window is already closed — there is nobody left to send output to.
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

/// Turns `pkexec`/`pacman` exit codes into messages users understand.
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
