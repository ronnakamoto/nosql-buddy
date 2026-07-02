# Release pipeline

`.github/workflows/release.yml` builds signed installers for macOS (two
native `.dmg`s — Apple Silicon and Intel, see note below), Windows
(`.msi` + NSIS `.exe`, unsigned for now), and Linux (`.deb` + `.AppImage`)
whenever a `v*` tag is pushed, and publishes them as a GitHub Release. The
updater manifest (`latest.json`) is generated and attached automatically so
the in-app "Check for updates" button (About screen) can find new versions.
This requires `bundle.createUpdaterArtifacts: true` in `tauri.conf.json` —
it defaults to `false` in Tauri v2 (see `tauri-utils`' `BundleConfig`), so
without it `cargo tauri build` silently skips generating the signed
`.tar.gz`/`.sig` updater bundles even with the updater plugin configured
and signing keys present.

**Why two DMGs instead of one universal binary:** mainly to keep the two
architectures' builds independent while the Rust toolchain pin below is in
place. Each architecture is built natively on its own runner (`macos-latest`
for Apple Silicon, `macos-15-intel` for Intel — the only Intel image GitHub
still offers).

**Rust toolchain is pinned to 1.88.0** (not `stable`) across all four build
jobs. Rust 1.89+ mangles the `__rust_probestack` symbol that `wasmer_vm` v4.x
(pulled in transitively via `ark-circom`) still depends on unmangled. This
only affects x86/x86_64 (LLVM only implements stack probes for those
architectures), so it breaks Windows, Linux, and Intel-macOS builds with
"undefined symbol: __rust_probestack" at link time — Apple Silicon is
unaffected but pinned too for consistency. `notify-rust` is downgraded to
4.17.0 in `Cargo.lock` (`cargo update -p notify-rust --precise 4.17.0`) in
the same change, since 4.18.0 requires rustc >= 1.89. Bump the toolchain
pin (and re-allow `notify-rust` to float) once `ark-circom`/`wasmer` are
upgraded past the fix (wasmer >= 6, see
https://github.com/wasmerio/wasmer/pull/5690).

The `zk-audit/soroban-contract` wasm build (a separate, isolated-workspace
crate — see its own `Cargo.toml`) needs rustc >= 1.89 (soroban-sdk 25.x),
which conflicts with the 1.88.0 pin above. It's built with its own `stable`
toolchain (`cargo +stable`), installed alongside the pinned one in the same
job, rather than sharing the app's toolchain.

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
