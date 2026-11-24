#!/usr/bin/env bash
# Installs, updates or removes Setu for the current user on x86_64 Linux:
#   curl -fsSL https://raw.githubusercontent.com/sudharamachandranrajeev/setu-sync-manager/main/install-linux.sh | bash
#
# What it does:
#   1. Installs the libraries Setu's window needs (WebKitGTK, GTK 3, the tray library) with
#      your package manager: pacman, apt or dnf. Asks for your password (sudo).
#   2. Installs Tailscale if it is missing, starts it, and if this computer is not signed in
#      yet, prints a link to sign in.
#   3. Downloads the newest Setu release and checks it against the release's .sha256 file.
#   4. Runs that release's install.sh: Setu goes into ~/.local/bin with a menu entry and a
#      start-at-login entry. An older Setu is stopped and replaced; its background helper
#      (0.1.0-alpha.4 and before) is removed. Then Setu opens.
#
# Run it again to update. Your rules, settings, this computer's sync identity and your synced
# files are never touched.
#
# Options:
#   --no-deps        skip steps 1 and 2 (you install the libraries and Tailscale yourself)
#   --no-autostart   do not start Setu at login
#   --uninstall      remove Setu (keeps your rules, settings and files)
set -euo pipefail

# Everything runs from main, so a download cut off halfway runs nothing.
main() {
  local repo=sudharamachandranrajeev/setu-sync-manager
  local bin=$HOME/.local/bin
  local autostart=${XDG_CONFIG_HOME:-$HOME/.config}/autostart/setu.desktop
  local deps=1 uninstall=0 opts=()

  for arg in "$@"; do
    case $arg in
      --no-deps) deps=0 ;;
      --no-autostart) opts+=(--no-autostart) ;;
      --uninstall) uninstall=1 ;;
      *) die "unknown option $arg (use --no-deps, --no-autostart or --uninstall)" ;;
    esac
  done

  [[ $(uname -m) == x86_64 ]] || die "Setu is only built for x86_64 Linux so far"
  for tool in curl tar sha256sum; do
    command -v "$tool" >/dev/null || die "$tool is needed; install it and run this again"
  done

  local sudo=
  ((EUID == 0)) || sudo=sudo

  if ((deps && !uninstall)); then
    install_libraries
    install_tailscale
  fi

  say "Finding the newest Setu"
  local api tarball name tmp
  api=$(curl -fsSL "https://api.github.com/repos/$repo/releases?per_page=20")
  tarball=$(grep -o "https://[^\"]*/releases/download/v[^\"]*-linux-x64\.tar\.gz" <<<"$api" | head -n1)
  [[ -n $tarball ]] || die "no Linux download found on the releases page"
  name=${tarball##*/}

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  say "Downloading ${name%-linux-x64.tar.gz}"
  curl -fL --progress-bar -o "$tmp/$name" "$tarball"
  curl -fsSL -o "$tmp/$name.sha256" "$tarball.sha256"
  (cd "$tmp" && sha256sum -c --quiet "$name.sha256") || die "the download is damaged (checksum does not match); run this again"
  tar -xzf "$tmp/$name" -C "$tmp"
  local src=$tmp/${name%.tar.gz}
  [[ -f $src/install.sh ]] || die "the download has no install.sh"

  if ((uninstall)); then
    bash "$src/install.sh" --uninstall
    exit 0
  fi

  # An update keeps the person's choice: Setu was installed and start-at-login is off.
  if [[ -x $bin/setu-app && ! -f $autostart ]]; then
    opts+=(--no-autostart)
  fi
  # On a desktop, open Setu straight away (an older one running is stopped first).
  if [[ -n ${WAYLAND_DISPLAY:-}${DISPLAY:-} ]]; then
    opts+=(--restart)
  fi
  say "Installing"
  bash "$src/install.sh" "${opts[@]}"

  say "Done. Setu is in your app menu (on Omarchy: Super + Space, then type Setu)."
  case ":$PATH:" in *":$bin:"*) ;; *) say "To use the setu command in a terminal, add $bin to your PATH." ;; esac
}

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

install_libraries() {
  if command -v pacman >/dev/null; then
    say "Installing the libraries Setu needs (pacman)"
    $sudo pacman -S --needed --noconfirm webkit2gtk-4.1 gtk3 libayatana-appindicator
  elif command -v apt-get >/dev/null; then
    say "Installing the libraries Setu needs (apt)"
    $sudo apt-get update -qq
    $sudo apt-get install -y libwebkit2gtk-4.1-0 libayatana-appindicator3-1 libgtk-3-0t64 ||
      $sudo apt-get install -y libwebkit2gtk-4.1-0 libayatana-appindicator3-1 libgtk-3-0
  elif command -v dnf >/dev/null; then
    say "Installing the libraries Setu needs (dnf)"
    $sudo dnf install -y webkit2gtk4.1 gtk3 libayatana-appindicator-gtk3
  else
    say "Unknown package manager: install WebKitGTK 4.1, GTK 3 and libayatana-appindicator yourself"
  fi
}

# Setu finds your other computers only over Tailscale.
install_tailscale() {
  if ! command -v tailscale >/dev/null; then
    if command -v pacman >/dev/null; then
      say "Installing Tailscale (pacman)"
      $sudo pacman -S --needed --noconfirm tailscale
    else
      say "Installing Tailscale (tailscale.com/install.sh)"
      curl -fsSL https://tailscale.com/install.sh | sh
    fi
  fi
  if command -v systemctl >/dev/null && ! systemctl is-active --quiet tailscaled; then
    say "Starting Tailscale"
    $sudo systemctl enable --now tailscaled
  fi
  if ! tailscale ip -4 >/dev/null 2>&1; then
    say "Sign this computer in to Tailscale (same account as your other computers): open the link below"
    $sudo tailscale up
  fi
}

main "$@"
