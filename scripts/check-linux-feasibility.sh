#!/usr/bin/env bash
# Runs on a native Linux desktop to prove the prerequisites for a future backend.

set -euo pipefail

if [[ "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Usage: ./scripts/check-linux-feasibility.sh

Run this on a native X11 or Wayland desktop. It checks the active session,
builds Flash Shot for the host, and verifies the platform services that a
future screenshot backend must use. See docs/linux-platform-validation.md for
the required manual capture, clipboard, shortcut, and multi-display checks.
EOF
  exit 0
fi

if ! command -v cargo >/dev/null; then
  echo "cargo is required for the Linux feasibility check" >&2
  exit 1
fi

session_type="${XDG_SESSION_TYPE:-unknown}"
case "$session_type" in
  wayland)
    if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
      echo "XDG_SESSION_TYPE is wayland but WAYLAND_DISPLAY is unset" >&2
      exit 1
    fi
    if ! command -v busctl >/dev/null; then
      echo "busctl is required to verify the xdg-desktop-portal service" >&2
      exit 1
    fi
    busctl --user status org.freedesktop.portal.Desktop >/dev/null
    busctl --user introspect org.freedesktop.portal.Desktop \
      /org/freedesktop/portal/desktop org.freedesktop.portal.ScreenCast >/dev/null
    busctl --user introspect org.freedesktop.portal.Desktop \
      /org/freedesktop/portal/desktop org.freedesktop.portal.GlobalShortcuts >/dev/null
    ;;
  x11)
    if [[ -z "${DISPLAY:-}" ]]; then
      echo "XDG_SESSION_TYPE is x11 but DISPLAY is unset" >&2
      exit 1
    fi
    if ! command -v xdpyinfo >/dev/null; then
      echo "xdpyinfo is required to verify the active X11 server" >&2
      exit 1
    fi
    xdpyinfo >/dev/null
    ;;
  *)
    echo "Run this from a native X11 or Wayland desktop session; found '$session_type'." >&2
    exit 1
    ;;
esac

cargo check --workspace --all-targets
echo "Linux $session_type prerequisites and host compilation passed."
