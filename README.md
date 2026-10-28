# ArchSwift

**English** | [Русский](README.ru.md)

**ArchSwift** is an open-source graphical wizard for installing local Arch Linux
packages (`*.pkg.tar.zst`). Double-clicking a package opens a familiar
step-by-step installer in the spirit of a Windows setup wizard:

1. **Welcome** — name, version, architecture, dependencies, size, signature and
   other details from `.PKGINFO`.
2. **Installation** — the native Polkit dialog (KDE Auth / GNOME Polkit), then
   `pkexec pacman -U --noconfirm <package>` on a worker thread. The interface
   never freezes and the pacman journal is streamed in real time.
3. **Finish** — success with a “Finish” button, or a clear error with the last
   journal lines and a “Retry” button.

ArchSwift is written in Rust with GTK4 and targets Arch Linux and derivatives
that ship pacman and Polkit.

Repository: <https://github.com/watsbunny/ArchSwift>.

## Features

- Safe archive parsing without unpacking the whole payload.
- External commands are spawned without a shell: the package path can never
  become a command.
- Dedicated threads for metadata reading and installation, with a channel as the
  only bridge to GTK.
- Three interface languages: English, Russian and Belarusian. The language is
  detected from the user’s locale.
- A dedicated MIME type, `application/x-arch-swift-package`, so packages do not
  open in an archiver.
- Icons, a `.desktop` file and pacman hooks that refresh the MIME, desktop and
  hicolor icon caches right after installation.
- Protection against closing the window while pacman is running.

## Project layout

```
.
├── Cargo.toml                     # dependencies and the release profile
├── Cargo.lock                     # reproducible builds
├── build.rs                       # GResource with icons, translation registry
├── src/
│   ├── main.rs                    # entry point, application ID, message channel
│   ├── ui.rs                      # three-step GTK4 wizard
│   ├── metadata.rs                # safe `.PKGINFO` reading from the archive
│   ├── installer.rs               # pkexec + pacman on a worker thread
│   └── i18n.rs                    # loading and selecting TOML translations
├── localization/lang/             # en.toml, ru.toml, be.toml
├── assets/
│   ├── icons/                     # SVG icons for the app and the file type
│   └── resources.gresource.xml    # packing icons into a GResource
├── arch-swift.xml                 # MIME type application/x-arch-swift-package
├── arch-swift.desktop             # launcher and MIME association
├── arch-swift.install             # post_install/post_upgrade/post_remove
└── PKGBUILD                       # AUR-style package build
```

## Dependencies

To build:

- Rust 1.80+ and Cargo;
- GTK4 with development headers (`gtk4`);
- `glib-compile-resources` from `glib2` (required by `build.rs`);
- `pkg-config` and `xz` (to build the `xz2` crate);
- the usual `base-devel` toolchain.

To run:

- `gtk4`, `polkit`, `pacman`;
- a desktop Polkit agent (KDE Polkit Auth Agent, GNOME Shell Polkit);
- `shared-mime-info`, `desktop-file-utils`, `hicolor-icon-theme` for proper
  desktop integration.

## Building and running without installing

```bash
cargo build --release
./target/release/arch-swift ./example-1.0-1-x86_64.pkg.tar.zst
```

Without an argument the wizard asks you to choose a package in a GTK file dialog.

Running the tests (including the check that all languages define the same keys):

```bash
cargo test
```

## Installing with PKGBUILD

Because the `PKGBUILD` sits next to the `src/` source directory, run `makepkg`
with a separate `BUILDDIR`. Otherwise makepkg uses `./src` as its extraction
directory and may delete the Rust sources:

```bash
BUILDDIR=/tmp/arch-swift-build makepkg -si
```

In a standalone AUR clone (where there is no `src/` directory) plain
`makepkg -si` is enough.

If you previously installed the `arch-linux-witchery` package (the former
project name), remove it first: `sudo pacman -Rns arch-linux-witchery`. The
package name, MIME type and application ID have changed, so the old and new
versions do not replace each other automatically.

A package built from the `v1.0.0` tag is also attached to the
[v1.0.0 release](https://github.com/watsbunny/ArchSwift/releases/tag/v1.0.0):

```bash
sudo pacman -U arch-swift-1.0.0-1-x86_64.pkg.tar.zst
```

When releasing a new version, update `pkgver` and the tag in `PKGBUILD` and run
`updpkgsums` to refresh the checksum.

After installing the package:

```bash
xdg-mime query default application/x-arch-swift-package
# arch-swift.desktop
```

If the association did not take effect in an already open KDE/GNOME session, set
it manually:

```bash
xdg-mime default arch-swift.desktop application/x-arch-swift-package
update-desktop-database ~/.local/share/applications 2>/dev/null || true
```

The package hooks automatically call `update-mime-database`,
`update-desktop-database` and `gtk-update-icon-cache` (or
`gtk4-update-icon-cache`) on install, upgrade and removal.

## Localization

All user-facing strings live in `localization/lang/*.toml`. Before compiling,
`build.rs` scans that directory, generates the `CATALOGS` language registry and
embeds the file contents into the binary with `include_str!`. Translation files
are never read at runtime, so the application stays self-contained.

Language selection priority:

1. `ARCHSWIFT_LANG` (handy for testing: `ARCHSWIFT_LANG=be`; an unknown code
   falls back to English and does not depend on the system locale);
2. `LANGUAGE` (colon-separated list);
3. `LC_ALL`, `LC_MESSAGES`, `LANG`;
4. English as the fallback.

The supported codes are `en`, `ru` and `be`; a locale such as `be_BY.UTF-8` is
detected correctly. If a key is missing in a translation, the English text is
used, and if it is missing there too, the key itself is shown. The
`all_catalogs_have_the_same_keys` test prevents forgetting a translation when a
new string is added.

### Adding a language

1. Create `localization/lang/<code>.toml` based on `en.toml`.
2. Run `cargo test` — the test verifies that the key set is complete.

`build.rs` picks up every `*.toml` file in `localization/lang` and embeds it, and
`match_locale` recognises the new code automatically. No Rust changes are
needed. For a regional variant, name the file `<code>-<REGION>.toml` (for
example `pt-BR.toml`): it is selected for the matching locale, while the base
`<code>.toml` stays as the fallback.

## Security model

- Metadata is read without root privileges; the path is canonicalized and the
  file type and extension are validated.
- `pkexec` is spawned through `std::process::Command` with an argument array and
  no `sh -c`, so file names are never interpreted by a shell.
- The administrator password is entered only in the system Polkit agent; the
  application neither reads nor stores it.
- `--disable-internal-agent` is used to prevent a text fallback from pkexec in a
  graphical session.
- The wizard warns that package install scripts run as root.
- Packages without a `.sig` file are not verified automatically; this is shown
  explicitly in the package card.

## License

MIT. See [LICENSE](LICENSE).
