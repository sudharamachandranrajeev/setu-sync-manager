#!/usr/bin/env bash
# Installs Setu for this user only (no sudo): the programs in ~/.local/bin, a menu entry,
# and the background helper as a systemd user service that starts at login.
#
#   ./install.sh               install or update
#   ./install.sh --no-helper   install without starting the background helper
#   ./install.sh --uninstall   remove the programs, menu entry and service
#
# Your rules, settings and synced files are never touched.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
helper=yes

have_systemd() { command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1; }

case "${1:-}" in
  --uninstall)
    if have_systemd; then systemctl --user disable --now setu-watch.service 2>/dev/null || true; fi
    rm -f "$bin/setu" "$bin/setu-watch" "$bin/setu-app" "$apps/setu.desktop" "$icons/setu.png" "$units/setu-watch.service"
    if have_systemd; then systemctl --user daemon-reload; fi
    echo "Setu removed. Your rules and settings are still in ~/.config/setu."
    exit 0
    ;;
  --no-helper) helper=no ;;
  "") ;;
  *) echo "unknown option: $1" >&2; exit 2 ;;
esac

for f in setu setu-watch setu-app setu.desktop setu.png setu-watch.service; do
  [[ -f "$here/$f" ]] || { echo "missing $f next to install.sh" >&2; exit 1; }
done

mkdir -p "$bin" "$apps" "$icons"
install -m 755 "$here/setu" "$here/setu-watch" "$here/setu-app" "$bin/"
install -m 644 "$here/setu.desktop" "$apps/setu.desktop"
install -m 644 "$here/setu.png" "$icons/setu.png"
echo "Installed setu, setu-watch and setu-app in $bin."

if [[ $helper == yes ]]; then
  if have_systemd; then
    mkdir -p "$units"
    install -m 644 "$here/setu-watch.service" "$units/setu-watch.service"
    systemctl --user daemon-reload
    systemctl --user enable setu-watch.service >/dev/null
    systemctl --user restart setu-watch.service
    echo "Background helper running; it starts at every login."
  else
    echo "No systemd user session: start $bin/setu-watch at login yourself."
  fi
fi

case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Note: $bin is not on your PATH. Add it to run setu from a terminal." ;;
esac
