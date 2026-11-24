#!/usr/bin/env bash
# Installs Setu for this user only (no sudo): the programs in ~/.local/bin, a menu entry, and
# an autostart entry so Setu starts (in the background) when you log in. Setu holds its own
# sync engine, so nothing else needs to run.
#
#   ./install.sh                  install or update
#   ./install.sh --no-autostart   install without starting Setu at login
#   ./install.sh --uninstall      remove the programs, menu entry and autostart entry
#   ./install.sh --restart        also start Setu again afterwards (in-app updates use it;
#                                 can be combined with --no-autostart)
#
# Your rules, settings, this computer's sync identity and your synced files are never touched.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps"
autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
doc="${XDG_DATA_HOME:-$HOME/.local/share}/doc/setu"
start_at_login=yes
restart=no

have_systemd() { command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1; }

# Ask a running Setu to quit, so its engine saves its index before the files change. Through
# `setu quit`: an older setu-app would ignore --quit and open its window.
stop_setu() {
  if [[ -x "$bin/setu" ]]; then "$bin/setu" quit >/dev/null 2>&1 || true; fi
}

# The background helper service of 0.1.0-alpha.4 and before; it now runs inside Setu.
remove_old_helper() {
  if have_systemd; then systemctl --user disable --now setu-watch.service 2>/dev/null || true; fi
  rm -f "$bin/setu-watch" "$units/setu-watch.service"
  if have_systemd; then systemctl --user daemon-reload; fi
}

for arg in "$@"; do
  case "$arg" in
    --uninstall)
      stop_setu
      remove_old_helper
      rm -f "$bin/setu" "$bin/setu-app" "$apps/setu.desktop" "$icons/setu.png" "$autostart/setu.desktop"
      rm -rf "$doc"
      echo "Setu removed. Your rules and settings are still in ~/.config/setu, and this computer's sync identity in ~/.local/state/setu."
      exit 0
      ;;
    --no-autostart) start_at_login=no ;;
    --restart) restart=yes ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

for f in setu setu-app setu.desktop setu.png LICENSE LICENSE-MPL-2.0 NOTICE; do
  [[ -f "$here/$f" ]] || { echo "missing $f next to install.sh" >&2; exit 1; }
done

stop_setu
remove_old_helper
mkdir -p "$bin" "$apps" "$icons" "$doc"
install -m 755 "$here/setu" "$here/setu-app" "$bin/"
install -m 644 "$here/setu.desktop" "$apps/setu.desktop"
install -m 644 "$here/setu.png" "$icons/setu.png"
install -m 644 "$here/LICENSE" "$here/LICENSE-MPL-2.0" "$here/NOTICE" "$doc/"
echo "Installed setu and setu-app in $bin."

if [[ $start_at_login == yes ]]; then
  mkdir -p "$autostart"
  cat > "$autostart/setu.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Setu
Comment=Keeps your folders in sync between your computers
Exec="$bin/setu-app" --background
Icon=setu
X-GNOME-Autostart-enabled=true
NoDisplay=true
EOF
  echo "Setu starts when you log in (turn it off in Setu's settings)."
else
  rm -f "$autostart/setu.desktop"
fi

if [[ $restart == yes ]]; then
  nohup "$bin/setu-app" >/dev/null 2>&1 &
  echo "Setu started again."
  exit 0
fi

case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Note: $bin is not on your PATH. Add it to run setu from a terminal." ;;
esac
echo "Start Setu from your applications menu."
