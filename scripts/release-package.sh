#!/usr/bin/env bash
# Build release tarballs into dist/ (darwin-arm64, linux-amd64, linux-arm64).
#
# All three are built here on the Mac: darwin natively, the Linux pair
# through cargo-zigbuild, which supplies a cross linker and glibc
# headers so the bundled SQLite's C sources compile for the target.
set -euo pipefail
source "$(dirname "$0")/release-common.sh"

VERSION="$(release_version)"
DIST="$ROOT/dist"
STAGE="$DIST/stage"
CARGO="$(rustup_cargo)"

rm -rf "$DIST"
mkdir -p "$STAGE"

build_one() {
	local triple=$1 asset_name=$2 builder=$3
	local asset="bullseye-${VERSION}-${asset_name}.tar.gz"

	echo "release-package: building ${triple} …" >&2
	"$CARGO" "$builder" --release --target "$triple"

	cp "target/${triple}/release/bullseye" "$STAGE/bullseye"
	tar -czf "$DIST/$asset" -C "$STAGE" bullseye -C "$ROOT" LICENSE README.md
	rm -f "$STAGE/bullseye"
	echo "wrote dist/$asset"
}

build_one aarch64-apple-darwin      darwin-arm64 build
build_one x86_64-unknown-linux-gnu  linux-amd64  zigbuild
build_one aarch64-unknown-linux-gnu linux-arm64  zigbuild

rmdir "$STAGE"

# Checksums are published alongside the tarballs so the tap can ingest
# real digests rather than skipping verification (🎯T74.5).
(cd "$DIST" && shasum -a 256 ./*.tar.gz > "bullseye-${VERSION}-SHA256SUMS")
echo "release-package: v${VERSION} → dist/"
ls -1 "$DIST"
