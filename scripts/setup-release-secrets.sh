#!/usr/bin/env bash
# One-time interactive setup of GitHub Actions secrets for .github/workflows/release.yml.
#
# Prompts for each credential locally (hidden input where sensitive) and
# pushes it straight to `gh secret set` — nothing is printed to stdout/logs.
#
# Requires: gh CLI authenticated (`gh auth status`), run from repo root.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v gh >/dev/null 2>&1; then
  echo "error: gh CLI not found. Install it from https://cli.github.com/" >&2
  exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
  echo "error: gh is not authenticated. Run 'gh auth login' first." >&2
  exit 1
fi

echo "Setting release-pipeline secrets on $(gh repo view --json nameWithOwner -q .nameWithOwner)"
echo

read -r -p "Path to Developer ID Application .p12 [$HOME/Documents/certificates-apple/NoSQLBuddy-DeveloperID.p12]: " P12_PATH
P12_PATH="${P12_PATH:-$HOME/Documents/certificates-apple/NoSQLBuddy-DeveloperID.p12}"
if [ ! -f "$P12_PATH" ]; then
  echo "error: file not found: $P12_PATH" >&2
  exit 1
fi

read -r -s -p "Password used when exporting that .p12: " P12_PASSWORD
echo
read -r -p "Apple signing identity [Developer ID Application: Adarsh Ron (66N65C2562)]: " SIGNING_IDENTITY
SIGNING_IDENTITY="${SIGNING_IDENTITY:-Developer ID Application: Adarsh Ron (66N65C2562)}"
read -r -p "Apple ID email (for notarization): " APPLE_ID_EMAIL
read -r -s -p "Apple app-specific password (appleid.apple.com -> App-Specific Passwords): " APPLE_APP_PASSWORD
echo
read -r -p "Apple Team ID [66N65C2562]: " APPLE_TEAM
APPLE_TEAM="${APPLE_TEAM:-66N65C2562}"

echo
echo "Setting Apple secrets..."
base64 -i "$P12_PATH" | gh secret set APPLE_CERTIFICATE
printf '%s' "$P12_PASSWORD" | gh secret set APPLE_CERTIFICATE_PASSWORD
printf '%s' "$SIGNING_IDENTITY" | gh secret set APPLE_SIGNING_IDENTITY
printf '%s' "$APPLE_ID_EMAIL" | gh secret set APPLE_ID
printf '%s' "$APPLE_APP_PASSWORD" | gh secret set APPLE_PASSWORD
printf '%s' "$APPLE_TEAM" | gh secret set APPLE_TEAM_ID
echo "Apple secrets set."
echo

echo "Now generating a fresh Tauri updater signing keypair"
echo "(the previous one's password was never saved, so we start clean)."
read -r -s -p "Choose a password to protect the new updater private key: " UPDATER_KEY_PASSWORD
echo
read -r -s -p "Confirm password: " UPDATER_KEY_PASSWORD_CONFIRM
echo
if [ "$UPDATER_KEY_PASSWORD" != "$UPDATER_KEY_PASSWORD_CONFIRM" ]; then
  echo "error: passwords did not match." >&2
  exit 1
fi

KEY_PATH="$HOME/.tauri/nosqlbuddy.key"
mkdir -p "$(dirname "$KEY_PATH")"
npx tauri signer generate --ci -w "$KEY_PATH" -p "$UPDATER_KEY_PASSWORD" -f

PUBKEY="$(cat "$KEY_PATH.pub")"
python3 - "$REPO_ROOT/src-tauri/tauri.conf.json" "$PUBKEY" <<'PY'
import json, sys
path, pubkey = sys.argv[1], sys.argv[2]
with open(path) as f:
    conf = json.load(f)
conf["plugins"]["updater"]["pubkey"] = pubkey
with open(path, "w") as f:
    json.dump(conf, f, indent=2)
    f.write("\n")
PY
echo "Updated plugins.updater.pubkey in src-tauri/tauri.conf.json (review and commit this change)."

echo "Setting updater secrets..."
cat "$KEY_PATH" | gh secret set TAURI_SIGNING_PRIVATE_KEY
printf '%s' "$UPDATER_KEY_PASSWORD" | gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
echo "Updater secrets set."
echo

echo "Done. Verify with: gh secret list"
echo "Keep a secure backup of $KEY_PATH and its password — losing it breaks auto-update signing permanently."
