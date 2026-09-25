# Maintainer: Pedro Henrique <13969802+pedrohfp@users.noreply.github.com>
pkgname=limpid
pkgver=0.1.0 # x-release-please-version
pkgrel=1
pkgdesc="System cleaner and disk-space analyser that understands btrfs snapshots, pacman and browser caches"
arch=('x86_64' 'aarch64')
url="https://github.com/heroesofcode/limpid"
license=('MIT')
depends=('gcc-libs' 'wayland' 'libxkbcommon' 'vulkan-icd-loader' 'polkit')
# paccache lives in pacman-contrib; without it the package-cache operation
# has nothing to run.
optdepends=(
  'pacman-contrib: trim the package cache'
  'docker: prune build cache and dangling images'
)
makedepends=('cargo')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR=target
  cargo build --frozen --release --workspace
}

check() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  cargo test --frozen --workspace
}

package() {
  cd "$pkgname-$pkgver"
  make DESTDIR="$pkgdir" PREFIX=/usr install
}
