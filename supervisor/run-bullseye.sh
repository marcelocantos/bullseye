#!/bin/sh
# Start `bullseye serve` for supervisord. Homebrew binary only on the
# shared port: a tree build there would serve unreleased tool schemas to
# every agent on the machine, which is the failure mode the Cellar-only
# rule exists to prevent.
set -e

if [ -z "${HOME:-}" ]; then
  HOME="$(eval echo ~"$(id -un)")"
  export HOME
fi
export USER="${USER:-$(id -un)}"
export PATH="/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:${HOME}/.cargo/bin:${HOME}/.local/bin:${HOME}/.py/bin:${HOME}/go/bin:/usr/bin:/bin:/usr/sbin:/sbin"
export BULLSEYE_ADDR="${BULLSEYE_ADDR:-127.0.0.1:18743}"

if [ -n "${BULLSEYE_BIN:-}" ]; then
  BIN="$BULLSEYE_BIN"
  if [ ! -x "$BIN" ]; then
    echo "bullseye: BULLSEYE_BIN=$BIN is not executable" >&2
    exit 1
  fi
else
  BIN="$(command -v bullseye 2>/dev/null || true)"
  if [ -z "$BIN" ] && command -v brew >/dev/null 2>&1; then
    BIN="$(brew --prefix)/opt/bullseye/bin/bullseye"
  fi
  if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
    echo "bullseye: no bullseye on PATH and no Homebrew install found." >&2
    echo "bullseye: brew install marcelocantos/tap/bullseye" >&2
    exit 1
  fi
fi

if [ "${1:-}" = "--print-bin" ]; then
  echo "$BIN"
  exit 0
fi

echo "bullseye: running $BIN serve (BULLSEYE_ADDR=$BULLSEYE_ADDR)" >&2
exec "$BIN" serve
