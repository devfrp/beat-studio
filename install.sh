#!/bin/sh
# Installs (or updates) Beat Studio for the current user on Linux x86_64:
#   curl -fsSL https://raw.githubusercontent.com/devfrp/beat-studio/main/install.sh | sh
# Removes it again:
#   curl -fsSL https://raw.githubusercontent.com/devfrp/beat-studio/main/install.sh | sh -s -- --uninstall
set -eu

REPO="devfrp/beat-studio"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
APP="$BIN_DIR/beatstudio"
DESKTOP="$DATA_DIR/applications/beatstudio.desktop"
ICON="$DATA_DIR/icons/hicolor/256x256/apps/beatstudio.png"

say() { printf '%s\n' "$*"; }
fail() { printf 'Beat Studio: %s\n' "$*" >&2; exit 1; }

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$APP" "$DESKTOP" "$ICON"
    say "Beat Studio removed. Your projects in ~/Documents/Beat Studio are kept."
    exit 0
fi

[ "$(uname -s)" = "Linux" ] || fail "this script is for Linux. Windows and macOS builds: https://github.com/$REPO/releases/latest"
[ "$(uname -m)" = "x86_64" ] || fail "only x86_64 is built for now ($(uname -m) here). You can build it from source: https://github.com/$REPO"

if command -v curl >/dev/null 2>&1; then
    get() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    get() { wget -q "$1" -O "$2"; }
else
    fail "needs curl or wget"
fi

mkdir -p "$BIN_DIR" "$(dirname "$DESKTOP")" "$(dirname "$ICON")"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

say "Downloading Beat Studio..."
get "https://github.com/$REPO/releases/latest/download/BeatStudio-x86_64.AppImage" "$TMP" || fail "download failed"
chmod +x "$TMP"
mv "$TMP" "$APP"
trap - EXIT

get "https://raw.githubusercontent.com/$REPO/main/packaging/beatstudio.png" "$ICON" || say "(no icon: download failed)"

cat > "$DESKTOP" <<EOF
[Desktop Entry]
Type=Application
Name=Beat Studio
Comment=Compose beats, melodies and songs
Exec=$APP
Icon=beatstudio
Terminal=false
Categories=AudioVideo;Audio;Midi;Sequencer;
StartupWMClass=beatstudio
EOF
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(dirname "$DESKTOP")" 2>/dev/null || true

say "Beat Studio is installed: $APP"
case ":$PATH:" in
    *":$BIN_DIR:"*) say "Start it with: beatstudio (or from your applications menu)" ;;
    *) say "Start it from your applications menu, or add $BIN_DIR to your PATH." ;;
esac

# AppImages need FUSE 2 to start; without it they can still run unpacked.
if ! ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; then
    say ""
    say "Note: libfuse2 seems missing. Install it (Debian/Ubuntu: libfuse2, Arch: fuse2),"
    say "or start Beat Studio with: APPIMAGE_EXTRACT_AND_RUN=1 $APP"
fi
