# Release pipeline

`.github/workflows/release.yml` builds signed installers for macOS
(universal `.dmg`), Windows (`.msi` + NSIS `.exe`, unsigned for now), and
Linux (`.deb` + `.AppImage`) whenever a `v*` tag is pushed, and publishes
them as a GitHub Release. The updater manifest (`latest.json`) is generated
and attached automatically so the in-app "Check for updates" button
(About screen) can find new versions.

## One-time setup: GitHub Secrets

Add these under **Settings → Secrets and variables → Actions**:

| Secret | How to get it |
| --- | --- |
| `APPLE_CERTIFICATE` | Export your **Developer ID Application** certificate (with private key) from Keychain Access as a `.p12`, then `base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | The password you set when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | The certificate's common name, e.g. `Developer ID Application: Your Name (TEAMID)` — find it with `security find-identity -v -p codesigning` |
| `APPLE_ID` | Your Apple ID email |
| `APPLE_PASSWORD` | An **app-specific password** for that Apple ID, created at appleid.apple.com → Sign-In and Security → App-Specific Passwords |
| `APPLE_TEAM_ID` | Your Team ID from the Apple Developer portal (Membership page) |
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the updater private key (see below) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for that private key |

### Updater signing key

A keypair was generated locally for this repo:

- Private key: `~/.tauri/nosqlbuddy.key` (on the machine it was generated on — **not committed**, back it up somewhere safe; losing it means old app installs can never verify a future signed update again)
- Public key: already placed in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`

To populate the GitHub secrets, run on the machine that has the key:

```bash
cat ~/.tauri/nosqlbuddy.key            # -> TAURI_SIGNING_PRIVATE_KEY
```

The key's password should be stored as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

If you ever need a fresh keypair (e.g. the old one was lost), regenerate it
and update `pubkey` in `tauri.conf.json`:

```bash
npx tauri signer generate -w ~/.tauri/nosqlbuddy.key
```

Note: rotating the key invalidates updates for everyone on an older
version — they'll need to reinstall manually once.

## Cutting a release

1. Bump the version in **all three** places so they match:
   - `src-tauri/tauri.conf.json` (`version`)
   - `src-tauri/Cargo.toml` (`package.version`)
   - `package.json` (`version`)
2. Commit, then tag and push:
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```
3. The `release` workflow creates a draft release, builds all three
   platforms, uploads signed installers + `latest.json`, then publishes the
   release once every platform succeeds.

## Known limitations

- **Windows builds are unsigned.** No code-signing certificate is
  configured yet, so Windows SmartScreen will show an "unknown publisher"
  warning. Add `WINDOWS_CERTIFICATE` / `WINDOWS_CERTIFICATE_PASSWORD` (or a
  certificate-store thumbprint) and wire them into the workflow once a
  certificate is available.
- **Linux packages are unsigned** (no code-signing convention exists for
  `.deb`/`.AppImage`); consider publishing GPG-signed checksums alongside
  the release if that becomes a requirement.
