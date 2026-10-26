#!/bin/sh
# Installs the latest Mouthpiece into /Applications.
#
#   curl -fsSL https://jfinchy.github.io/mouthpiece/install.sh | sh
#
# It downloads the same Mouthpiece.dmg as the website's download button,
# straight from GitHub Releases, checks the app's signature, and copies it
# into Applications. Because curl (unlike a browser) doesn't mark the file
# as downloaded, macOS doesn't show its "can't verify" prompt on first
# launch. Nothing else is changed on your Mac.
#
# Options (environment variables):
#   DW_APP_DIR=~/Applications   install somewhere other than /Applications
#   DW_NO_OPEN=1                don't open the app afterwards

set -eu

URL="https://github.com/JFinchy/mouthpiece/releases/latest/download/Mouthpiece.dmg"
APP_NAME="Mouthpiece.app"
APP_DIR="${DW_APP_DIR:-/Applications}"

fail() {
  printf 'Mouthpiece: %s\n' "$1" >&2
  exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "this installer is for macOS."
[ "$(uname -m)" = "arm64" ] || fail "Mouthpiece needs a Mac with Apple Silicon (M1 or later)."

tmp=$(mktemp -d)
mounted=""
cleanup() {
  [ -n "$mounted" ] && hdiutil detach "$tmp/mnt" -quiet >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

echo "Downloading Mouthpiece…"
curl -fL --progress-bar -o "$tmp/Mouthpiece.dmg" "$URL" || fail "the download failed."

hdiutil attach -nobrowse -readonly -quiet -mountpoint "$tmp/mnt" "$tmp/Mouthpiece.dmg" \
  || fail "couldn't open the downloaded disk image."
mounted=1

app="$tmp/mnt/$APP_NAME"
[ -d "$app" ] || fail "the disk image doesn't contain $APP_NAME."
codesign --verify --deep --strict "$app" 2>/dev/null || fail "the app's signature check failed, so it wasn't installed."

mkdir -p "$APP_DIR" 2>/dev/null || true
# Without admin rights /Applications is read-only; use the user's own
# Applications folder instead (unless a folder was asked for).
if [ ! -w "$APP_DIR" ] && [ -z "${DW_APP_DIR:-}" ]; then
  APP_DIR="$HOME/Applications"
  mkdir -p "$APP_DIR"
  echo "No write access to /Applications, so installing into ~/Applications."
fi
[ -w "$APP_DIR" ] || fail "can't write to $APP_DIR."
dest="$APP_DIR/$APP_NAME"

# Mouthpiece used to be called Dev Whisper.
old_name="$APP_DIR/Dev Whisper.app"
for running in "$dest" "$old_name"; do
  if pgrep -f "$running/Contents/MacOS/" >/dev/null 2>&1; then
    fail "Mouthpiece is running. Quit it from the menu bar, then run this again."
  fi
done

# Copy the new app in beside the old one, then swap them, so a step macOS
# blocks never leaves a half-deleted app behind.
staged="$APP_DIR/.Mouthpiece.installing.app"
rm -rf "$staged"
ditto "$app" "$staged" || fail "copying into $APP_DIR failed."
if [ -e "$dest" ] && ! mv "$dest" "$tmp/previous.app" 2>/dev/null; then
  rm -rf "$staged"
  fail "macOS didn't let your terminal replace the Mouthpiece already in $APP_DIR.
Its App Management setting protects installed apps. Either drag that copy
of Mouthpiece to the Trash and run this again, or allow your terminal in
System Settings > Privacy & Security > App Management."
fi
if ! mv "$staged" "$dest"; then
  [ -e "$tmp/previous.app" ] && mv "$tmp/previous.app" "$dest"
  rm -rf "$staged"
  fail "couldn't move the new app into $APP_DIR; the old one is unchanged."
fi
echo "Installed $dest"
# Your settings and history move over on first launch.
if [ -e "$old_name" ]; then
  if mv "$old_name" "$tmp/old-name.app" 2>/dev/null; then
    echo "Removed the old Dev Whisper app."
  else
    echo "Drag the old Dev Whisper app in $APP_DIR to the Trash; Mouthpiece replaces it."
  fi
fi

if [ -z "${DW_NO_OPEN:-}" ]; then
  open "$dest"
  echo "Opening Mouthpiece. The setup checklist in Settings walks you through the rest."
fi
