#!/usr/bin/env bash
# Build distributable single binaries locally and package them into dist/.
#
# The host target always builds with plain cargo. Other targets need a
# rustup-managed std for that target plus a cross linker — easiest via
# cargo-zigbuild (`brew install zig && cargo install cargo-zigbuild`) or
# `cross` (Docker). Targets whose toolchain isn't available are skipped.
# CI (.github/workflows/release.yml) builds the full matrix on native runners;
# this is for local/manual builds.
#
# Usage: dist/release.sh [target ...]
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=cta-tui
VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"
HOST="$(rustc -vV | sed -n 's/host: //p')"
OUT="dist/out"
mkdir -p "$OUT"

TARGETS=("$@")
if [ ${#TARGETS[@]} -eq 0 ]; then
  TARGETS=(
    aarch64-apple-darwin
    x86_64-apple-darwin
    x86_64-unknown-linux-musl
    aarch64-unknown-linux-musl
    x86_64-pc-windows-msvc
  )
fi

builder_for() { # echoes the build command prefix for a target, or "skip:<reason>"
  local t="$1"
  if [ "$t" = "$HOST" ]; then echo "cargo build"; return; fi
  if command -v cargo-zigbuild >/dev/null 2>&1; then echo "cargo zigbuild"; return; fi
  if command -v cross >/dev/null 2>&1; then echo "cross build"; return; fi
  echo "skip:no cross toolchain (install cargo-zigbuild or cross)"
}

built=()
for t in "${TARGETS[@]}"; do
  cmd="$(builder_for "$t")"
  if [[ "$cmd" == skip:* ]]; then
    echo "→ $t: skipped — ${cmd#skip:}"
    continue
  fi
  echo "→ $t: $cmd"
  rustup target add "$t" >/dev/null 2>&1 || true
  $cmd --release --target "$t"
  ext=""; [[ "$t" == *windows* ]] && ext=".exe"
  src="target/$t/release/$BIN$ext"
  [ -f "$src" ] || { echo "  ! missing $src"; continue; }
  name="$BIN-v$VERSION-$t"
  if [[ "$t" == *windows* ]]; then
    (cd "target/$t/release" && zip -q - "$BIN$ext") > "$OUT/$name.zip"
    pkg="$OUT/$name.zip"
  else
    tar -C "target/$t/release" -czf "$OUT/$name.tar.gz" "$BIN"
    pkg="$OUT/$name.tar.gz"
  fi
  built+=("$t")
  echo "  packaged $pkg"
done

# macOS universal binary when both arches built.
arm="target/aarch64-apple-darwin/release/$BIN"
x86="target/x86_64-apple-darwin/release/$BIN"
if [ -f "$arm" ] && [ -f "$x86" ] && command -v lipo >/dev/null 2>&1; then
  uni="$OUT/$BIN-universal"
  lipo -create -output "$uni" "$arm" "$x86"
  tar -C "$OUT" -czf "$OUT/$BIN-v$VERSION-universal-apple-darwin.tar.gz" "$(basename "$uni")"
  rm -f "$uni"
  echo "  packaged $OUT/$BIN-v$VERSION-universal-apple-darwin.tar.gz"
fi

# Checksums for everything we produced.
shopt -s nullglob
pkgs=("$OUT"/*.tar.gz "$OUT"/*.zip)
if [ ${#pkgs[@]} -gt 0 ]; then
  (cd "$OUT" && shasum -a 256 *.tar.gz *.zip 2>/dev/null > SHA256SUMS.txt)
  echo "Wrote $OUT/SHA256SUMS.txt"
fi
echo "Done. Built: ${built[*]:-(host only / none)}"
