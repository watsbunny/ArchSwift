# Maintainer: ArchSwift Team <maintainers@openarch.dev>
#
# WARNING: this PKGBUILD sits next to the src/ source directory. Running
# makepkg without a separate BUILDDIR makes it use ./src as the extraction
# directory, which can delete the Rust sources. Use:
#   BUILDDIR=/tmp/arch-swift-build makepkg -si
#
# Building from source. To test the project locally without publishing a
# release, point source at a local archive and update sha256sums, or build the
# binary directly with `cargo build --release`.

pkgname=arch-swift
pkgver=1.0.0
pkgrel=1
pkgdesc="Wizard-style graphical installer for local Arch Linux packages"
arch=('x86_64' 'aarch64')
url="https://github.com/watsbunny/ArchSwift"
license=('MIT')
depends=(
  'gtk4'
  'polkit'
  'pacman'
  'hicolor-icon-theme'
  'shared-mime-info'
  'desktop-file-utils'
)
makedepends=('cargo' 'rust' 'xz' 'pkgconf')
# Rust already does thin LTO (profile.release). makepkg's -flto=auto flag is
# added to CFLAGS and breaks linking of bundled zstd (rust-lld cannot read GCC
# LTO bitcode), so LTO is disabled for the C parts.
options=('!lto')
optdepends=('xdg-utils: manual file association setup')
install="${pkgname}.install"
# The checksum matches the v1.0.0 tag archive on GitHub.
# When releasing a new version, update pkgver/the tag and run `updpkgsums`.
source=("${pkgname}-${pkgver}.tar.gz::${url}/archive/v${pkgver}.tar.gz")
sha256sums=('4475faf93b573c270e8ff5f3a0c9ca4a700d200ff96d89ca2c827f5053e6fbb5')

build() {
  cd "ArchSwift-${pkgver}"

  export CARGO_TARGET_DIR="${srcdir}/target"
  export CARGO_HOME="${srcdir}/cargo-home"
  # Strip build-directory absolute paths from panic messages and debug info.
  export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=${srcdir}=/usr/src/${pkgname} --remap-path-prefix=${startdir}=/usr/src/${pkgname}"

  cargo build --release --locked
}

check() {
  cd "ArchSwift-${pkgver}"

  export CARGO_TARGET_DIR="${srcdir}/target"
  export CARGO_HOME="${srcdir}/cargo-home"
  # Strip build-directory absolute paths from panic messages and debug info.
  export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=${srcdir}=/usr/src/${pkgname} --remap-path-prefix=${startdir}=/usr/src/${pkgname}"

  cargo test --release --locked
}

package() {
  cd "ArchSwift-${pkgver}"

  install -Dm755 "${srcdir}/target/release/arch-swift" \
    "${pkgdir}/usr/bin/arch-swift"

  install -Dm644 arch-swift.desktop \
    "${pkgdir}/usr/share/applications/arch-swift.desktop"
  install -Dm644 arch-swift.xml \
    "${pkgdir}/usr/share/mime/packages/arch-swift.xml"

  install -Dm644 assets/icons/arch-swift-installer.svg \
    "${pkgdir}/usr/share/icons/hicolor/scalable/apps/arch-swift-installer.svg"
  install -Dm644 assets/icons/arch-swift-package-icon.svg \
    "${pkgdir}/usr/share/icons/hicolor/scalable/apps/arch-swift-package-icon.svg"

  # Translation sources are installed for reference: they are already embedded in
  # the binary via include_str!, but translators like to have them at hand.
  for lang_file in localization/lang/*.toml; do
    install -Dm644 "${lang_file}" \
      "${pkgdir}/usr/share/arch-swift/lang/$(basename "${lang_file}")"
  done

  install -Dm644 README.md \
    "${pkgdir}/usr/share/doc/${pkgname}/README.md"
  install -Dm644 README.ru.md \
    "${pkgdir}/usr/share/doc/${pkgname}/README.ru.md"
  install -Dm644 LICENSE \
    "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}
