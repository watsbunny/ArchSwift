# Maintainer: ArchSwift Team <maintainers@openarch.dev>
#
# ВНИМАНИЕ: этот PKGBUILD лежит рядом с каталогом исходников src/. Если
# запустить makepkg без отдельного BUILDDIR, makepkg будет использовать ./src
# как каталог распаковки и может удалить исходники. Используйте:
#   BUILDDIR=/tmp/arch-swift-build makepkg -si
#
# Сборка из исходников. Для локальной проверки проекта без публикации релиза
# замените source на локальный архив и обновите sha256sums, либо соберите
# бинарник напрямую командой `cargo build --release`.

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
# Rust сам делает thin LTO (profile.release). Флаг -flto=auto от makepkg
# добавляется к CFLAGS и ломает линковку bundled zstd (GCC LTO bitcode
# не понимает rust-lld), поэтому для C-частей LTO отключаем.
options=('!lto')
optdepends=('xdg-utils: ручная настройка ассоциации файлов')
install="${pkgname}.install"
# Контрольная сумма соответствует архиву тега v1.0.0 на GitHub.
# При выпуске новой версии обновите pkgver/тег и выполните `updpkgsums`.
source=("${pkgname}-${pkgver}.tar.gz::${url}/archive/v${pkgver}.tar.gz")
sha256sums=('4475faf93b573c270e8ff5f3a0c9ca4a700d200ff96d89ca2c827f5053e6fbb5')

build() {
  cd "ArchSwift-${pkgver}"

  export CARGO_TARGET_DIR="${srcdir}/target"
  export CARGO_HOME="${srcdir}/cargo-home"
  # Убираем абсолютные пути сборочного каталога из panic-сообщений и debug info.
  export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=${srcdir}=/usr/src/${pkgname} --remap-path-prefix=${startdir}=/usr/src/${pkgname}"

  cargo build --release --locked
}

check() {
  cd "ArchSwift-${pkgver}"

  export CARGO_TARGET_DIR="${srcdir}/target"
  export CARGO_HOME="${srcdir}/cargo-home"
  # Убираем абсолютные пути сборочного каталога из panic-сообщений и debug info.
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

  # Исходники переводов устанавливаются для справки: в бинарник они уже
  # встроены через include_str!, но переводчикам удобно иметь их под рукой.
  for lang_file in localization/lang/*.toml; do
    install -Dm644 "${lang_file}" \
      "${pkgdir}/usr/share/arch-swift/lang/$(basename "${lang_file}")"
  done

  install -Dm644 README.md \
    "${pkgdir}/usr/share/doc/${pkgname}/README.md"
  install -Dm644 LICENSE \
    "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}
