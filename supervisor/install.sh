#!/bin/sh
# Render supervisor/bullseye.ini into supervisor.d and make supervisord
# the owner of :18743. Exactly one parent may own that port, so evict
# brew services / launchd / any stray listener before starting.
set -e

REPO="$(CDPATH= cd "$(dirname "$0")/.." && pwd)"
CONF_DIR="${SUPERVISOR_CONF_DIR:-/opt/homebrew/etc/supervisor.d}"
DEST="$CONF_DIR/bullseye.ini"
TEMPLATE="$REPO/supervisor/bullseye.ini"
PORT="${BULLSEYE_PORT:-18743}"

if [ -z "${HOME:-}" ]; then
  HOME="$(eval echo ~"$(id -un)")"
  export HOME
fi

mkdir -p "$CONF_DIR"
mkdir -p "$HOME/.local/var/log"
chmod +x "$REPO/supervisor/run-bullseye.sh" "$REPO/supervisor/install.sh"

rm -f "$DEST"
sed "s|@REPO@|$REPO|g" "$TEMPLATE" >"$DEST"
echo "rendered $DEST (from $TEMPLATE)"

if [ "${SUPERVISOR_SKIP_CTL:-}" = 1 ]; then
  exit 0
fi

if ! command -v supervisorctl >/dev/null 2>&1; then
  echo "supervisorctl not on PATH — ini written; start Homebrew supervisor to load it" >&2
  exit 1
fi

if command -v brew >/dev/null 2>&1; then
  brew services stop bullseye >/dev/null 2>&1 || true
fi
if command -v launchctl >/dev/null 2>&1; then
  launchctl bootout "gui/$(id -u)/homebrew.mxcl.bullseye" 2>/dev/null || true
fi

if command -v lsof >/dev/null 2>&1; then
  holder="$(lsof -nP -iTCP:"$PORT" -sTCP:LISTEN -t 2>/dev/null || true)"
  holder="$(printf '%s\n' "$holder" | head -n 1)"
  if [ -n "$holder" ]; then
    echo "bullseye: stopping pid $holder still holding :$PORT"
    kill "$holder" 2>/dev/null || true
    i=0
    while [ "$i" -lt 20 ] && kill -0 "$holder" 2>/dev/null; do
      sleep 1
      i=$((i + 1))
    done
    kill -9 "$holder" 2>/dev/null || true
  fi
fi

supervisorctl reread
supervisorctl update
supervisorctl restart bullseye 2>/dev/null || supervisorctl start bullseye

echo "bullseye installed at $DEST"
supervisorctl status bullseye
