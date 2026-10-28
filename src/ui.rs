//! GTK4 interface of the three-step wizard.
//!
//! Thread safety: all background operations talk to the UI only through
//! `std::sync::mpsc`. Every 80 ms the main thread drains messages and updates
//! widgets. That is simpler and safer than passing GTK objects between threads.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use gtk::prelude::*;
use gtk::Box as GtkBox;
use gtk::{
    gio, glib, Align, Application, ApplicationWindow, Button, Expander, Frame, Grid, HeaderBar,
    Image, Label, Orientation, ProgressBar, ScrolledWindow, Spinner, Stack, StackTransitionType,
    TextBuffer, TextView, Window,
};

use crate::installer::{self, InstallOutcome};
use crate::metadata::{self, PackageInfo};
use crate::{i18n, WorkerMsg, APP_NAME, ICON_APP};

/// Lifecycle phases of the window. They drive button labels and sensitivity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Loading,
    Ready,
    Installing,
    Succeeded,
    Failed,
    LoadError,
}

/// Mutable state that is not a widget.
struct State {
    sender: Sender<WorkerMsg>,
    receiver: Receiver<WorkerMsg>,
    path: Option<PathBuf>,
    package: Option<PackageInfo>,
    phase: Phase,
    /// Last journal lines — used in the error message.
    last_lines: VecDeque<String>,
}

impl State {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            path: None,
            package: None,
            phase: Phase::Idle,
            last_lines: VecDeque::new(),
        }
    }
}

/// All widgets touched by handlers and background messages.
#[derive(Clone)]
struct Wizard {
    window: ApplicationWindow,
    stack: Stack,
    back: Button,
    close: Button,
    next: Button,

    choose: Button,
    meta_spinner: Spinner,
    meta_status: Label,
    fields: MetaFields,

    install_caption: Label,
    install_status: Label,
    install_last_line: Label,
    install_spinner: Spinner,
    progress: ProgressBar,
    log_buffer: TextBuffer,
    log_view: TextView,
    log_expander: Expander,

    finish_icon: Image,
    finish_title: Label,
    finish_body: Label,
    finish_detail: Label,
    finish_log: Expander,
}

/// Metadata field labels. A separate struct makes resetting on load easier.
#[derive(Clone)]
struct MetaFields {
    name: Label,
    version: Label,
    arch: Label,
    description: Label,
    url: Label,
    packager: Label,
    build_date: Label,
    installed_size: Label,
    file_size: Label,
    licenses: Label,
    depends: Label,
    optdepends: Label,
    provides: Label,
    conflicts: Label,
    replaces: Label,
    groups: Label,
    signature: Label,
    install_script: Label,
    path: Label,
}

impl MetaFields {
    fn new() -> Self {
        Self {
            name: value_label(false),
            version: value_label(false),
            arch: value_label(false),
            description: value_label(false),
            url: value_label(false),
            packager: value_label(false),
            build_date: value_label(false),
            installed_size: value_label(false),
            file_size: value_label(false),
            licenses: value_label(false),
            depends: value_label(false),
            optdepends: value_label(false),
            provides: value_label(false),
            conflicts: value_label(false),
            replaces: value_label(false),
            groups: value_label(false),
            signature: value_label(false),
            install_script: value_label(false),
            path: value_label(true),
        }
    }

    fn reset(&self) {
        for label in [
            &self.name,
            &self.version,
            &self.arch,
            &self.description,
            &self.url,
            &self.packager,
            &self.build_date,
            &self.installed_size,
            &self.file_size,
            &self.licenses,
            &self.depends,
            &self.optdepends,
            &self.provides,
            &self.conflicts,
            &self.replaces,
            &self.groups,
            &self.signature,
            &self.install_script,
            &self.path,
        ] {
            label.set_text("—");
        }
    }
}

/// Creates the wizard window and connects every handler.
pub fn build(application: &Application, initial_path: Option<PathBuf>) {
    register_bundled_icons();
    install_css();
    // The window icon comes from the installed hicolor theme. If the package is
    // not installed yet, the same icon is taken from the bundled GResource.
    Window::set_default_icon_name(ICON_APP);

    let window = ApplicationWindow::builder()
        .application(application)
        .title(APP_NAME)
        .default_width(900)
        .default_height(740)
        .build();

    window.set_titlebar(Some(&build_header_bar()));

    let stack = Stack::builder()
        .transition_type(StackTransitionType::SlideLeftRight)
        .transition_duration(220)
        .vexpand(true)
        .build();

    let (welcome_page, fields, meta_spinner, meta_status, choose) = build_welcome_page();
    let log_buffer = TextBuffer::new(None);
    let (
        progress_page,
        progress,
        install_caption,
        install_status,
        install_last_line,
        install_spinner,
        log_view,
        log_expander,
    ) = build_progress_page(&log_buffer);
    let (finish_page, finish_icon, finish_title, finish_body, finish_detail, finish_log) =
        build_finish_page(&log_buffer);

    stack.add_named(&welcome_page, Some("welcome"));
    stack.add_named(&progress_page, Some("progress"));
    stack.add_named(&finish_page, Some("finish"));

    let back = Button::with_label(&t("button.back"));
    let close = Button::with_label(&t("button.close"));
    let next = Button::with_label(&t("button.next"));
    next.add_css_class("suggested-action");

    let actions = GtkBox::new(Orientation::Horizontal, 8);
    actions.add_css_class("action-bar");
    actions.append(&back);

    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    actions.append(&spacer);
    actions.append(&close);
    actions.append(&next);

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.append(&stack);
    root.append(&actions);
    window.set_child(Some(&root));

    let wizard = Rc::new(Wizard {
        window,
        stack,
        back,
        close,
        next,
        choose,
        meta_spinner,
        meta_status,
        fields,
        install_caption,
        install_status,
        install_last_line,
        install_spinner,
        progress,
        log_buffer,
        log_view,
        log_expander,
        finish_icon,
        finish_title,
        finish_body,
        finish_detail,
        finish_log,
    });

    let state = Rc::new(RefCell::new(State::new()));

    connect_signals(&wizard, &state);
    start_message_polling(&wizard, &state);
    start_progress_animation(&wizard);
    set_idle_metadata_state(&wizard);
    refresh_buttons(&wizard, Phase::Idle);

    wizard.window.present();

    if let Some(path) = initial_path {
        load_package(&wizard, &state, path);
    } else {
        // Show the dialog only after the window has been drawn once, so that KDE and
        // GNOME position it correctly relative to its parent.
        let wizard_for_dialog = wizard.clone();
        let state_for_dialog = state.clone();
        glib::idle_add_local_once(move || {
            choose_package(&wizard_for_dialog, &state_for_dialog);
        });
    }
}

/// Window header with the title and subtitle.
fn build_header_bar() -> HeaderBar {
    let header = HeaderBar::new();

    let title_box = GtkBox::new(Orientation::Vertical, 0);
    title_box.set_valign(Align::Center);

    let title = Label::new(Some(APP_NAME));
    title.add_css_class("title");
    let subtitle = Label::new(Some(&t("app.subtitle")));
    subtitle.add_css_class("subtitle");

    title_box.append(&title);
    title_box.append(&subtitle);
    header.set_title_widget(Some(&title_box));

    header
}

/// Step 1: welcome screen and package details.
fn build_welcome_page() -> (ScrolledWindow, MetaFields, Spinner, Label, Button) {
    let content = GtkBox::new(Orientation::Vertical, 18);
    content.set_margin_top(26);
    content.set_margin_bottom(26);
    content.set_margin_start(28);
    content.set_margin_end(28);

    let hero = GtkBox::new(Orientation::Horizontal, 18);
    let app_icon = Image::from_icon_name(ICON_APP);
    app_icon.set_pixel_size(72);
    app_icon.set_valign(Align::Start);

    let hero_text = GtkBox::new(Orientation::Vertical, 4);
    let heading = Label::new(Some(&t("welcome.heading")));
    heading.add_css_class("wizard-heading");
    heading.set_xalign(0.0);
    heading.set_wrap(true);

    let intro = Label::new(Some(&t("welcome.intro")));
    intro.set_xalign(0.0);
    intro.set_wrap(true);
    intro.add_css_class("dim-label");

    hero_text.append(&heading);
    hero_text.append(&intro);
    hero.append(&app_icon);
    hero.append(&hero_text);
    content.append(&hero);

    let fields = MetaFields::new();
    let frame = Frame::new(Some(&t("welcome.section_title")));
    let grid = Grid::builder()
        .column_spacing(22)
        .row_spacing(10)
        .margin_top(14)
        .margin_bottom(14)
        .margin_start(14)
        .margin_end(14)
        .build();

    let mut row = 0;
    add_meta_row(&grid, row, &t("meta.pkgname"), &fields.name);
    row += 1;
    add_meta_row(&grid, row, &t("meta.version"), &fields.version);
    row += 1;
    add_meta_row(&grid, row, &t("meta.arch"), &fields.arch);
    row += 1;
    add_meta_row(&grid, row, &t("meta.description"), &fields.description);
    row += 1;
    add_meta_row(&grid, row, &t("meta.url"), &fields.url);
    row += 1;
    add_meta_row(&grid, row, &t("meta.packager"), &fields.packager);
    row += 1;
    add_meta_row(&grid, row, &t("meta.build_date"), &fields.build_date);
    row += 1;
    add_meta_row(
        &grid,
        row,
        &t("meta.installed_size"),
        &fields.installed_size,
    );
    row += 1;
    add_meta_row(&grid, row, &t("meta.file_size"), &fields.file_size);
    row += 1;
    add_meta_row(&grid, row, &t("meta.licenses"), &fields.licenses);
    row += 1;
    add_meta_row(&grid, row, &t("meta.depends"), &fields.depends);
    row += 1;
    add_meta_row(&grid, row, &t("meta.optdepends"), &fields.optdepends);
    row += 1;
    add_meta_row(&grid, row, &t("meta.provides"), &fields.provides);
    row += 1;
    add_meta_row(&grid, row, &t("meta.conflicts"), &fields.conflicts);
    row += 1;
    add_meta_row(&grid, row, &t("meta.replaces"), &fields.replaces);
    row += 1;
    add_meta_row(&grid, row, &t("meta.groups"), &fields.groups);
    row += 1;
    add_meta_row(&grid, row, &t("meta.signature"), &fields.signature);
    row += 1;
    add_meta_row(
        &grid,
        row,
        &t("meta.install_script"),
        &fields.install_script,
    );
    row += 1;
    add_meta_row(&grid, row, &t("meta.path"), &fields.path);

    frame.set_child(Some(&grid));
    content.append(&frame);

    let warning = GtkBox::new(Orientation::Vertical, 4);
    warning.add_css_class("warning-box");
    let warning_title = Label::new(Some(&t("welcome.warning_title")));
    warning_title.add_css_class("warning-title");
    warning_title.set_xalign(0.0);
    let warning_text = Label::new(Some(&t("welcome.warning_text")));
    warning_text.set_xalign(0.0);
    warning_text.set_wrap(true);
    warning.append(&warning_title);
    warning.append(&warning_text);
    content.append(&warning);

    let choose_row = GtkBox::new(Orientation::Horizontal, 12);
    let choose = Button::with_label(&t("button.choose_package"));
    let choose_hint = Label::new(Some(&t("welcome.choose_hint")));
    choose_hint.set_xalign(0.0);
    choose_hint.set_wrap(true);
    choose_hint.add_css_class("dim-label");
    choose_row.append(&choose);
    choose_row.append(&choose_hint);
    content.append(&choose_row);

    let status_row = GtkBox::new(Orientation::Horizontal, 10);
    let meta_spinner = Spinner::new();
    meta_spinner.set_visible(false);
    let meta_status = Label::new(Some(&t("welcome.not_selected")));
    meta_status.set_xalign(0.0);
    meta_status.set_wrap(true);
    meta_status.set_hexpand(true);
    status_row.append(&meta_spinner);
    status_row.append(&meta_status);
    content.append(&status_row);

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .child(&content)
        .build();

    (scroll, fields, meta_spinner, meta_status, choose)
}

/// Step 2: installation progress and the pacman journal.
#[allow(clippy::type_complexity)]
fn build_progress_page(
    log_buffer: &TextBuffer,
) -> (
    GtkBox,
    ProgressBar,
    Label,
    Label,
    Label,
    Spinner,
    TextView,
    Expander,
) {
    let page = GtkBox::new(Orientation::Vertical, 16);
    page.set_margin_top(32);
    page.set_margin_bottom(24);
    page.set_margin_start(32);
    page.set_margin_end(32);

    let heading = Label::new(Some(&t("progress.heading")));
    heading.add_css_class("wizard-heading");
    heading.set_xalign(0.0);
    heading.set_wrap(true);

    let caption = Label::new(Some(&t("common.dash")));
    caption.add_css_class("package-caption");
    caption.set_xalign(0.0);
    caption.set_selectable(true);
    caption.set_wrap(true);

    let auth_card = GtkBox::new(Orientation::Horizontal, 12);
    auth_card.add_css_class("install-card");
    let lock_icon = Image::from_icon_name("system-lock-screen-symbolic");
    lock_icon.set_pixel_size(28);
    lock_icon.set_valign(Align::Start);
    let auth_text = Label::new(Some(&t("progress.auth")));
    auth_text.set_wrap(true);
    auth_text.set_xalign(0.0);
    auth_text.set_hexpand(true);
    auth_card.append(&lock_icon);
    auth_card.append(&auth_text);

    let progress = ProgressBar::new();
    progress.set_show_text(true);
    progress.set_text(Some(&t("progress.progress_waiting")));
    progress.set_pulse_step(0.06);
    progress.add_css_class("install-progress");

    let status_row = GtkBox::new(Orientation::Horizontal, 10);
    let install_spinner = Spinner::new();
    install_spinner.start();
    let install_status = Label::new(Some(&t("progress.preparing")));
    install_status.set_xalign(0.0);
    install_status.set_wrap(true);
    install_status.set_hexpand(true);
    status_row.append(&install_spinner);
    status_row.append(&install_status);

    let install_last_line = Label::new(Some(""));
    install_last_line.set_xalign(0.0);
    install_last_line.set_ellipsize(gtk::pango::EllipsizeMode::End);
    install_last_line.add_css_class("dim-label");
    install_last_line.set_width_chars(40);
    install_last_line.set_max_width_chars(110);

    let log_view = TextView::new();
    log_view.set_buffer(Some(log_buffer));
    log_view.set_editable(false);
    log_view.set_cursor_visible(false);
    log_view.set_monospace(true);
    log_view.set_wrap_mode(gtk::WrapMode::WordChar);
    log_view.set_left_margin(10);
    log_view.set_right_margin(10);
    log_view.set_top_margin(8);
    log_view.set_bottom_margin(8);
    log_view.add_css_class("log-view");

    let log_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(220)
        .child(&log_view)
        .build();

    let log_expander = Expander::new(Some(&t("progress.details")));
    log_expander.set_child(Some(&log_scroll));
    log_expander.set_expanded(false);

    page.append(&heading);
    page.append(&caption);
    page.append(&auth_card);
    page.append(&progress);
    page.append(&status_row);
    page.append(&install_last_line);
    page.append(&log_expander);

    (
        page,
        progress,
        caption,
        install_status,
        install_last_line,
        install_spinner,
        log_view,
        log_expander,
    )
}

/// Step 3: success or failure.
#[allow(clippy::type_complexity)]
fn build_finish_page(log_buffer: &TextBuffer) -> (GtkBox, Image, Label, Label, Label, Expander) {
    let outer = GtkBox::new(Orientation::Vertical, 0);

    let inner = GtkBox::new(Orientation::Vertical, 14);
    inner.set_halign(Align::Center);
    inner.set_valign(Align::Center);
    inner.set_vexpand(true);
    inner.set_margin_top(28);
    inner.set_margin_bottom(28);
    inner.set_margin_start(36);
    inner.set_margin_end(36);

    let icon = Image::from_icon_name("emblem-ok-symbolic");
    icon.set_pixel_size(96);
    icon.add_css_class("success-icon");

    let title = Label::new(Some(&t("finish.success_title")));
    title.add_css_class("finish-title");
    title.set_justify(gtk::Justification::Center);
    title.set_wrap(true);

    let body = Label::new(Some(""));
    body.set_justify(gtk::Justification::Center);
    body.set_wrap(true);
    body.set_max_width_chars(72);
    body.set_xalign(0.5);

    let detail = Label::new(Some(""));
    detail.set_wrap(true);
    detail.set_selectable(true);
    detail.set_xalign(0.0);
    detail.set_max_width_chars(84);
    detail.add_css_class("error-text");
    detail.set_visible(false);

    let finish_log_view = TextView::new();
    finish_log_view.set_buffer(Some(log_buffer));
    finish_log_view.set_editable(false);
    finish_log_view.set_cursor_visible(false);
    finish_log_view.set_monospace(true);
    finish_log_view.set_wrap_mode(gtk::WrapMode::WordChar);
    finish_log_view.set_left_margin(10);
    finish_log_view.set_right_margin(10);
    finish_log_view.set_top_margin(8);
    finish_log_view.set_bottom_margin(8);
    finish_log_view.add_css_class("log-view");

    let log_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(180)
        .child(&finish_log_view)
        .build();

    let finish_log = Expander::new(Some(&t("finish.details")));
    finish_log.set_child(Some(&log_scroll));
    finish_log.set_expanded(false);
    finish_log.set_visible(false);

    inner.append(&icon);
    inner.append(&title);
    inner.append(&body);
    inner.append(&detail);
    inner.append(&finish_log);
    outer.append(&inner);

    (outer, icon, title, body, detail, finish_log)
}

/// Connects the buttons and prevents closing the window during installation.
fn connect_signals(wizard: &Rc<Wizard>, state: &Rc<RefCell<State>>) {
    {
        let state = state.clone();
        let wizard_for_click = wizard.clone();
        let button = wizard.next.clone();
        button.connect_clicked(move |_| {
            let phase = state.borrow().phase;
            match phase {
                Phase::Ready | Phase::Failed => start_install(&wizard_for_click, &state),
                Phase::Succeeded => wizard_for_click.window.close(),
                Phase::Idle | Phase::Loading | Phase::Installing | Phase::LoadError => {}
            }
        });
    }

    {
        let state = state.clone();
        let wizard_for_click = wizard.clone();
        let button = wizard.back.clone();
        button.connect_clicked(move |_| {
            if !matches!(state.borrow().phase, Phase::Failed) {
                return;
            }

            {
                let mut current = state.borrow_mut();
                current.phase = Phase::Ready;
            }

            let title = state
                .borrow()
                .package
                .as_ref()
                .map(|package| i18n::tr("app.window_package", &[("package", &package.pkgname)]))
                .unwrap_or_else(|| APP_NAME.to_string());

            wizard_for_click.window.set_title(Some(&title));
            wizard_for_click.stack.set_visible_child_name("welcome");
            refresh_buttons(&wizard_for_click, Phase::Ready);
        });
    }

    {
        let state = state.clone();
        let wizard_for_click = wizard.clone();
        let button = wizard.close.clone();
        button.connect_clicked(move |_| {
            if !matches!(state.borrow().phase, Phase::Installing) {
                wizard_for_click.window.close();
            }
        });
    }

    {
        let state = state.clone();
        let wizard_for_click = wizard.clone();
        let button = wizard.choose.clone();
        button.connect_clicked(move |_| {
            choose_package(&wizard_for_click, &state);
        });
    }

    {
        let state = state.clone();
        let window_weak = wizard.window.downgrade();
        wizard.window.connect_close_request(move |_| {
            if matches!(state.borrow().phase, Phase::Installing) {
                if let Some(window) = window_weak.upgrade() {
                    let dialog = gtk::AlertDialog::builder()
                        .modal(true)
                        .message(t("dialog.close_running_title"))
                        .detail(t("dialog.close_running_detail"))
                        .build();
                    dialog.show(Some(&window));
                }
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
        });
    }
}

/// Periodically drains messages from background threads on the GTK thread.
fn start_message_polling(wizard: &Rc<Wizard>, state: &Rc<RefCell<State>>) {
    let wizard = wizard.clone();
    let state = state.clone();

    glib::timeout_add_local(Duration::from_millis(80), move || {
        let mut batch = Vec::new();

        {
            let current = state.borrow();
            while batch.len() < 256 {
                match current.receiver.try_recv() {
                    Ok(message) => batch.push(message),
                    Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }

        for message in batch {
            handle_worker_message(&wizard, &state, message);
        }

        glib::ControlFlow::Continue
    });
}

/// Animates the indeterminate progress while the second step is visible.
fn start_progress_animation(wizard: &Rc<Wizard>) {
    let progress = wizard.progress.clone();

    glib::timeout_add_local(Duration::from_millis(120), move || {
        if progress.is_mapped() {
            progress.pulse();
        }
        glib::ControlFlow::Continue
    });
}

/// Handles a single message from a background thread.
fn handle_worker_message(wizard: &Rc<Wizard>, state: &Rc<RefCell<State>>, message: WorkerMsg) {
    match message {
        WorkerMsg::Metadata(Ok(package)) => {
            {
                let mut current = state.borrow_mut();
                current.package = Some((*package).clone());
                current.phase = Phase::Ready;
            }

            fill_metadata(wizard, &package);
            wizard.meta_spinner.stop();
            wizard.meta_spinner.set_visible(false);
            wizard.meta_status.remove_css_class("error-text");
            wizard.meta_status.add_css_class("dim-label");
            wizard.meta_status.set_text(&i18n::tr(
                "welcome.ready",
                &[("package", &package.summary())],
            ));
            wizard.window.set_title(Some(&i18n::tr(
                "app.window_package",
                &[("package", &package.pkgname)],
            )));
            wizard.stack.set_visible_child_name("welcome");
            refresh_buttons(wizard, Phase::Ready);

            log_info!(
                "Metadata loaded: {} ({}).",
                package.summary(),
                package.path.display()
            );
        }

        WorkerMsg::Metadata(Err(error)) => {
            {
                let mut current = state.borrow_mut();
                current.package = None;
                current.phase = Phase::LoadError;
            }

            wizard.meta_spinner.stop();
            wizard.meta_spinner.set_visible(false);
            wizard.meta_status.remove_css_class("dim-label");
            wizard.meta_status.add_css_class("error-text");
            wizard
                .meta_status
                .set_text(&i18n::tr("welcome.error", &[("error", &error)]));
            wizard.window.set_title(Some(APP_NAME));
            wizard.stack.set_visible_child_name("welcome");
            refresh_buttons(wizard, Phase::LoadError);

            log_info!("Metadata error: {error}");
        }

        WorkerMsg::Log(line) => {
            let line = line.trim_end();
            if line.trim().is_empty() {
                return;
            }

            append_log(wizard, line);

            {
                let mut current = state.borrow_mut();
                if current.last_lines.len() >= 50 {
                    current.last_lines.pop_front();
                }
                current.last_lines.push_back(line.to_string());
            }

            wizard.install_last_line.set_text(line);
            update_install_status(wizard, line);
        }

        WorkerMsg::Finished(outcome) => {
            finish_install(wizard, state, outcome);
        }
    }
}

/// Starts reading metadata in a separate thread.
fn load_package(wizard: &Rc<Wizard>, state: &Rc<RefCell<State>>, path: PathBuf) {
    {
        let mut current = state.borrow_mut();
        if matches!(current.phase, Phase::Installing | Phase::Loading) {
            return;
        }
        current.phase = Phase::Loading;
        current.package = None;
        current.path = Some(path.clone());
        current.last_lines.clear();
    }

    wizard.fields.reset();
    wizard.stack.set_visible_child_name("welcome");
    wizard.meta_spinner.start();
    wizard.meta_spinner.set_visible(true);
    wizard.meta_status.remove_css_class("error-text");
    wizard.meta_status.add_css_class("dim-label");
    wizard.meta_status.set_text(&t("welcome.reading"));
    wizard.window.set_title(Some(APP_NAME));

    // Clear the traces of a previous installation so that "Retry" does not show
    // the journal of another package.
    wizard.log_buffer.set_text("");
    wizard.log_expander.set_expanded(false);
    wizard.finish_log.set_expanded(false);
    wizard.finish_log.set_visible(false);
    wizard.finish_detail.set_visible(false);
    wizard.progress.set_fraction(0.0);
    wizard
        .progress
        .set_text(Some(&t("progress.progress_waiting")));
    wizard.install_caption.set_text(&t("common.dash"));
    wizard.install_status.set_text(&t("progress.preparing"));
    wizard.install_last_line.set_text("");

    refresh_buttons(wizard, Phase::Loading);

    let sender = state.borrow().sender.clone();
    let fallback_sender = sender.clone();
    let thread_path = path.clone();

    let spawn_result = thread::Builder::new()
        .name("arch-swift-metadata".to_string())
        .spawn(move || {
            log_info!("Reading metadata: {}", thread_path.display());
            let result = metadata::load(&thread_path).map(Box::new);
            if sender.send(WorkerMsg::Metadata(result)).is_err() {
                log_info!("Metadata message receiver is already closed.");
            }
        });

    if let Err(error) = spawn_result {
        let _ = fallback_sender.send(WorkerMsg::Metadata(Err(i18n::tr(
            "error.metadata.thread",
            &[("error", &error.to_string())],
        ))));
    }
}

/// Starts installing the selected package.
fn start_install(wizard: &Rc<Wizard>, state: &Rc<RefCell<State>>) {
    let (path, package) = {
        let mut current = state.borrow_mut();
        if matches!(current.phase, Phase::Installing | Phase::Loading) {
            return;
        }

        let (Some(path), Some(package)) = (current.path.clone(), current.package.clone()) else {
            return;
        };

        current.phase = Phase::Installing;
        current.last_lines.clear();
        (path, package)
    };

    wizard.log_buffer.set_text("");
    wizard.log_expander.set_expanded(false);
    wizard.log_expander.set_visible(true);
    wizard.finish_log.set_expanded(false);
    wizard.finish_log.set_visible(false);
    wizard.progress.set_fraction(0.0);
    wizard.progress.set_text(Some(&t("progress.progress_auth")));
    wizard.install_caption.set_text(&package.summary());
    wizard
        .install_status
        .set_text(&t("progress.waiting_polkit"));
    wizard
        .install_last_line
        .set_text(&format!("pkexec pacman -U --noconfirm {}", path.display()));
    wizard.install_spinner.start();
    wizard.stack.set_visible_child_name("progress");
    wizard.window.set_title(Some(&i18n::tr(
        "app.window_install",
        &[("package", &package.pkgname)],
    )));
    refresh_buttons(wizard, Phase::Installing);

    log_info!(
        "Starting installation: pkexec pacman -U --noconfirm {}",
        path.display()
    );

    let sender = state.borrow().sender.clone();
    if let Err(error) = installer::start(&path, sender) {
        log_info!("Failed to start installation: {error}");
        finish_install(
            wizard,
            state,
            InstallOutcome {
                success: false,
                exit_code: None,
                error: Some(error),
            },
        );
    }
}

/// Shows the final screen and changes the purpose of the buttons.
fn finish_install(wizard: &Rc<Wizard>, state: &Rc<RefCell<State>>, outcome: InstallOutcome) {
    let package = state.borrow().package.clone();
    let last_lines: Vec<String> = state.borrow().last_lines.iter().cloned().collect();

    {
        let mut current = state.borrow_mut();
        current.phase = if outcome.success {
            Phase::Succeeded
        } else {
            Phase::Failed
        };
    }

    wizard.install_spinner.stop();
    wizard.finish_log.set_visible(true);
    wizard
        .finish_log
        .set_expanded(!outcome.success && !last_lines.is_empty());

    if outcome.success {
        wizard.progress.set_fraction(1.0);
        wizard.progress.set_text(Some(&t("progress.completed")));
        wizard.finish_icon.set_icon_name(Some("emblem-ok-symbolic"));
        set_finish_style(wizard, true);
        wizard.finish_title.set_text(&t("finish.success_title"));

        let summary = package
            .map(|package| package.summary())
            .unwrap_or_else(|| APP_NAME.to_string());
        wizard
            .finish_body
            .set_text(&i18n::tr("finish.success_body", &[("package", &summary)]));
        wizard.finish_detail.set_text("");
        wizard.finish_detail.set_visible(false);
        wizard.window.set_title(Some(&t("app.window_success")));

        log_info!("Installation completed successfully: {summary}.");
    } else {
        wizard.progress.set_text(Some(&t("progress.failed")));
        wizard
            .finish_icon
            .set_icon_name(Some("dialog-error-symbolic"));
        set_finish_style(wizard, false);
        wizard.finish_title.set_text(&t("finish.error_title"));

        let mut detail = outcome.error.unwrap_or_else(|| t("finish.error_default"));

        if !last_lines.is_empty() {
            detail.push_str("\n\n");
            detail.push_str(&t("finish.journal_tail"));
            detail.push('\n');
            for line in last_lines.iter().rev().take(4).rev() {
                detail.push_str("• ");
                detail.push_str(line);
                detail.push('\n');
            }
            detail.pop();
        }

        wizard.finish_body.set_text(&t("finish.error_body"));
        wizard.finish_detail.set_text(&detail);
        wizard.finish_detail.set_visible(true);
        wizard.window.set_title(Some(&t("app.window_error")));

        log_info!(
            "Installation failed: {} (code {:?}).",
            detail.lines().next().unwrap_or("unknown"),
            outcome.exit_code
        );
    }

    wizard.stack.set_visible_child_name("finish");
    refresh_buttons(wizard, state.borrow().phase);
}

/// Applies the colour scheme to the result icon.
fn set_finish_style(wizard: &Wizard, success: bool) {
    wizard.finish_icon.remove_css_class("success-icon");
    wizard.finish_icon.remove_css_class("error-icon");
    wizard.finish_icon.add_css_class(if success {
        "success-icon"
    } else {
        "error-icon"
    });
}

/// Opens the native package chooser dialog.
fn choose_package(wizard: &Rc<Wizard>, state: &Rc<RefCell<State>>) {
    if matches!(state.borrow().phase, Phase::Installing | Phase::Loading) {
        return;
    }

    let dialog = gtk::FileDialog::builder()
        .title(t("dialog.choose_title"))
        .modal(true)
        .build();

    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&t("dialog.filter_name")));
    filter.add_pattern("*.pkg.tar.zst");
    filter.add_pattern("*.pkg.tar.xz");
    filter.add_pattern("*.pkg.tar.gz");
    filter.add_pattern("*.pkg.tar");

    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));

    let window = wizard.window.clone();
    let wizard = wizard.clone();
    let state = state.clone();
    dialog.open(
        Some(&window),
        None::<&gio::Cancellable>,
        move |result| match result {
            Ok(file) => match file.path() {
                Some(path) => load_package(&wizard, &state, path),
                None => {
                    wizard.meta_status.add_css_class("error-text");
                    wizard
                        .meta_status
                        .set_text(&i18n::tr("dialog.not_local", &[]));
                    log_info!("The selected file has no local path.");
                }
            },
            Err(error) => {
                // The user closing the dialog is a normal situation.
                if error.matches(gtk::DialogError::Dismissed) {
                    log_info!("Package selection dialog was dismissed.");
                } else {
                    wizard.meta_status.add_css_class("error-text");
                    wizard.meta_status.set_text(&i18n::tr(
                        "dialog.file_dialog_closed",
                        &[("error", error.message())],
                    ));
                    log_info!("Package selection dialog failed: {}", error.message());
                }
            }
        },
    );
}

/// Fills the metadata card.
fn fill_metadata(wizard: &Wizard, package: &PackageInfo) {
    let dash = t("common.dash");
    let none = t("common.none");
    let no_description = t("common.no_description");
    let unknown = t("common.unknown");
    let no_licenses = t("common.no_licenses");
    let sig_found = t("meta.sig_found");
    let sig_missing = t("meta.sig_missing");
    let script_present = t("meta.script_present");
    let script_absent = t("meta.script_absent");

    wizard.fields.name.set_text(&package.pkgname);
    wizard.fields.version.set_text(&package.pkgver);
    wizard.fields.arch.set_text(if package.arch.is_empty() {
        &dash
    } else {
        &package.arch
    });
    wizard
        .fields
        .description
        .set_text(if package.pkgdesc.is_empty() {
            &no_description
        } else {
            &package.pkgdesc
        });
    wizard.fields.url.set_text(if package.url.is_empty() {
        &dash
    } else {
        &package.url
    });
    wizard
        .fields
        .packager
        .set_text(if package.packager.is_empty() {
            &unknown
        } else {
            &package.packager
        });

    let build_date = package.build_date().unwrap_or_else(|| t("common.unknown"));
    let installed_size = package
        .installed_size
        .map(metadata::human_size)
        .unwrap_or_else(|| t("common.not_specified"));

    wizard.fields.build_date.set_text(&build_date);
    wizard.fields.installed_size.set_text(&installed_size);
    wizard
        .fields
        .file_size
        .set_text(&metadata::human_size(package.file_size));
    wizard
        .fields
        .licenses
        .set_text(&join_or_none(&package.licenses, &no_licenses));
    wizard
        .fields
        .depends
        .set_text(&join_or_none(&package.depends, &none));
    wizard
        .fields
        .optdepends
        .set_text(&join_or_none(&package.optdepends, &none));
    wizard
        .fields
        .provides
        .set_text(&join_or_none(&package.provides, &none));
    wizard
        .fields
        .conflicts
        .set_text(&join_or_none(&package.conflicts, &none));
    wizard
        .fields
        .replaces
        .set_text(&join_or_none(&package.replaces, &none));
    wizard
        .fields
        .groups
        .set_text(&join_or_none(&package.groups, &none));
    wizard.fields.signature.set_text(if package.signed {
        &sig_found
    } else {
        &sig_missing
    });
    wizard
        .fields
        .install_script
        .set_text(if package.has_install_script {
            &script_present
        } else {
            &script_absent
        });
    wizard
        .fields
        .path
        .set_text(&package.path.display().to_string());
}

/// Joins repeated values with line breaks; an empty list becomes a placeholder.
fn join_or_none(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        empty.to_string()
    } else {
        items.join("\n")
    }
}

/// Adds a "key — value" row to the grid.
fn add_meta_row(grid: &Grid, row: i32, title: &str, value: &Label) {
    let key = Label::new(Some(title));
    key.set_xalign(0.0);
    key.set_halign(Align::Start);
    key.set_valign(Align::Start);
    key.add_css_class("meta-key");

    grid.attach(&key, 0, row, 1, 1);
    grid.attach(value, 1, row, 1, 1);
}

/// Creates a value label that wraps and can be selected.
fn value_label(monospace: bool) -> Label {
    let label = Label::new(Some("—"));
    label.set_xalign(0.0);
    label.set_halign(Align::Start);
    label.set_valign(Align::Start);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label.add_css_class("meta-value");

    if monospace {
        label.add_css_class("monospace");
    }

    label
}

/// The "no file selected yet" state.
fn set_idle_metadata_state(wizard: &Wizard) {
    wizard.meta_spinner.stop();
    wizard.meta_spinner.set_visible(false);
    wizard.meta_status.remove_css_class("error-text");
    wizard.meta_status.add_css_class("dim-label");
    wizard.meta_status.set_text(&t("welcome.not_selected"));
}

/// Updates labels and button sensitivity for the current phase.
fn refresh_buttons(wizard: &Wizard, phase: Phase) {
    wizard.back.set_visible(matches!(phase, Phase::Failed));
    wizard.back.set_sensitive(matches!(phase, Phase::Failed));
    wizard.back.set_label(&t("button.back_to_packages"));

    wizard.close.set_visible(!matches!(phase, Phase::Succeeded));
    wizard
        .close
        .set_sensitive(!matches!(phase, Phase::Installing));
    wizard.close.set_label(&t("button.close"));

    wizard.next.set_visible(true);
    wizard.next.set_sensitive(matches!(
        phase,
        Phase::Ready | Phase::Succeeded | Phase::Failed
    ));
    wizard.next.set_label(&match phase {
        Phase::Ready => t("button.next"),
        Phase::Installing => t("button.installing"),
        Phase::Succeeded => t("button.finish"),
        Phase::Failed => t("button.retry"),
        Phase::Loading => t("button.loading"),
        Phase::Idle | Phase::LoadError => t("button.next"),
    });

    wizard.choose.set_sensitive(!matches!(
        phase,
        Phase::Installing | Phase::Loading | Phase::Succeeded
    ));
}

/// Appends a line to the journal buffer and scrolls it to the bottom.
fn append_log(wizard: &Wizard, line: &str) {
    let mut end = wizard.log_buffer.end_iter();
    if wizard.log_buffer.char_count() > 0 {
        wizard.log_buffer.insert(&mut end, "\n");
    }
    wizard.log_buffer.insert(&mut end, line);

    let mut end = wizard.log_buffer.end_iter();
    let _ = wizard
        .log_view
        .scroll_to_iter(&mut end, 0.0, false, 0.0, 1.0);
}

/// Updates the progress caption from key pacman messages.
///
/// Pacman is localized together with the system, so both English and Russian
/// phrases are checked. If nothing matches, the status is left unchanged.
fn update_install_status(wizard: &Wizard, line: &str) {
    let lower = line.to_lowercase();

    let status = if lower.contains("checking dependencies")
        || lower.contains("проверка зависимостей")
    {
        t("progress.checking_deps")
    } else if lower.contains("resolving dependencies") || lower.contains("разрешение зависимостей")
    {
        t("progress.resolving_deps")
    } else if lower.contains("checking package integrity") || lower.contains("проверка целостности")
    {
        t("progress.checking_integrity")
    } else if lower.contains("loading package files") || lower.contains("загрузка файлов пакетов")
    {
        t("progress.loading_files")
    } else if lower.contains("installing ")
        || lower.contains("установка ")
        || lower.contains("upgrading ")
    {
        t("progress.installing_files")
    } else if lower.contains("running post-transaction hooks")
        || lower.contains("запуск post-transaction")
    {
        t("progress.hooks")
    } else if lower.contains("error")
        || lower.contains("failed")
        || lower.contains("ошибка")
        || lower.contains("не удалось")
    {
        t("progress.error")
    } else {
        return;
    };

    wizard.install_status.set_text(&status);
    wizard.progress.set_text(Some(&status));
}

/// Registers the bundled SVG icons in the hicolor theme of the current display.
///
/// The resources are registered in `main.rs`; here it is enough to tell GTK
/// where to look for the `scalable/apps/<name>.svg` directory.
fn register_bundled_icons() {
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_resource_path("/org/openarch/swift/icons");
    }
}

/// Loads the CSS once per display.
fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);

    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Small theme that does not depend on a specific KDE/GNOME colour scheme.
const CSS: &str = r#"
headerbar .title {
    font-weight: 700;
}

headerbar .subtitle {
    font-size: 0.82em;
    opacity: 0.7;
}

.wizard-heading {
    font-size: 22px;
    font-weight: 800;
}

.finish-title {
    font-size: 24px;
    font-weight: 800;
}

.package-caption {
    font-size: 15px;
    font-weight: 700;
}

.meta-key {
    opacity: 0.68;
    font-size: 0.92em;
}

.meta-value {
    font-weight: 500;
}

.warning-box {
    background-color: rgba(245, 194, 17, 0.13);
    border: 1px solid rgba(245, 194, 17, 0.40);
    border-radius: 12px;
    padding: 14px;
}

.warning-title {
    font-weight: 700;
}

.install-card {
    background-color: rgba(127, 127, 127, 0.08);
    border-radius: 12px;
    padding: 16px;
}

.install-progress {
    min-height: 18px;
}

.action-bar {
    border-top: 1px solid rgba(127, 127, 127, 0.25);
    padding: 12px 18px;
}

.success-icon {
    color: #2ec27e;
}

.error-icon {
    color: #e01b24;
}

.error-text {
    color: #e01b24;
}

.log-view {
    font-size: 0.9em;
}
"#;

/// Short helper for strings without placeholders.
fn t(key: &str) -> String {
    i18n::tr(key, &[])
}
