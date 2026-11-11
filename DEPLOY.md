# Librarium: building and releasing

Releases are built, signed and published by GitHub Actions when a `v*` tag is pushed to this repository, **`myeti/librarium`**, which is public. The installers and `latest.json` (the in-app updater manifest) are attached to its own GitHub release: nothing is pushed to another repository. You never build or sign a release on your machine.

## Cutting a release

```bash
npm run bump 0.6.0                 # or: npm run bump -- --patch | --minor | --major
git commit -m "Bump 0.6.0" -- package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock
npm run release                    # checks, asks for the release notes, tags, pushes
```

**`npm run bump`** sets the version in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock`, and prints what changed. It refuses versions that aren't `X.Y.Z` (MSI installers reject pre-release suffixes) or aren't higher than the current one. `src-tauri/tauri.conf.json` isn't touched: it reads its version from `package.json` (`"version": "../package.json"`). Nothing is committed unless you add `--commit` (`npm run bump -- 0.6.0 --commit`).

**`npm run release`** refuses to run unless:
- every copy of the version agrees;
- you are on `main` with no uncommitted changes to tracked files;
- `main` contains `origin/main` after a fetch. It may be ahead (the bump commit), never behind or diverged;
- the tag `vX.Y.Z` doesn't exist locally or on `origin` (exact name), and is higher than every existing `v*` tag.

It then opens your editor with the commits since the last tag. Turn them into **release notes for users** (Markdown is fine; lines starting with `;` are dropped; an empty message cancels). The notes become the annotated tag's message, which CI publishes on the GitHub release page and in `latest.json` (the app shows them in its update prompt). After a confirmation it pushes `main` and the tag together (`git push --atomic`). Use `--notes-file FILE` to skip the editor, `--dry-run` to run the checks only, and `--yes` to skip the confirmation.

Then follow the **Release** workflow in the Actions tab. It takes about 20–30 minutes.

## What CI does

### On every push and pull request to `main` or `overhaul/**` (`.github/workflows/ci.yml`)

On macOS (arm64), Windows and Ubuntu 24.04: `npm ci`, `vue-tsc`, `eslint src`, `vitest`, the IPC contract check, the version check, a frontend build, then `cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked`. A separate job runs `cargo deny check` with `deny.toml` (at the repository root, where cargo-deny finds it without `--config`): known vulnerabilities, licences (permissive licences + MPL-2.0; the GPL-3.0-or-later crate `boko` is listed as an explicit exception — the GPL `epub` crate is gone since the EPUB module replaced it; any *new* copyleft crate fails), sources (crates.io only) and duplicate versions (warning).

Not enabled yet: `cargo fmt --check`. The code isn't rustfmt-formatted; make one formatting-only commit, then add the step.

### On a `v*` tag (`.github/workflows/release.yml`)

| Job | What it does | Secrets it sees |
|---|---|---|
| `check` | Tag = `v` + `package.json` version = `Cargo.toml` = `Cargo.lock`; the tagged commit is on `main`; the version isn't already published; the release notes = the annotated tag's message (a lightweight or empty tag fails here). | — |
| `build` (×4) | Unsigned builds, in parallel: macOS Apple Silicon and macOS Intel (`app`, `dmg`), Windows x64 (NSIS `.exe` and `.msi`), Linux x64 (`.AppImage`, `.deb`). `npm ci`, `cargo --locked`, toolchain from `rust-toolchain.toml`, no caches. Uploads the installers and, separately, the debug symbols. | `VITE_*` only |
| `sign` | Signs the five updater artifacts (two `.app.tar.gz`, the `.exe`, the `.msi`, the `.AppImage`) with `tauri signer sign --app-version X.Y.Z`, then verifies every signature against the public key in `tauri.conf.json`. Runs no project dependency code: only Node, our own scripts and the Tauri CLI, installed alone at the `package-lock.json` version with install scripts disabled. | signing key + password |
| `publish` | Writes `latest.json` and `SHA256SUMS`, verifies them, creates a **draft** release for the tag on this repository (or reuses an existing draft), uploads everything, downloads it back from the draft and verifies it again, then publishes it and marks it latest. The only job allowed to write (`permissions: contents: write`), with the workflow's own token. | — |

Why it is split like this: the build jobs run hundreds of npm packages (and the frontend build), so the updater key is never available there. Before, every npm dependency could read it during the build.

The workflow has no manual trigger on purpose: a run on a branch would publish a `latest.json` for a version like `main` and break updates for everyone. A concurrency group stops two releases from running at once.

**macOS: two builds rather than one universal binary.** Each user downloads (and updates) half the bytes, the two builds run in parallel, and each updater entry points to its own architecture. Intel can be dropped later (macOS 26 is the last release for Intel Macs) by deleting one matrix entry. The minimum macOS version is 10.15 (`bundle.macOS.minimumSystemVersion`).

**What `latest.json` contains.** The updater looks up `{os}-{arch}-{bundle}` first, then `{os}-{arch}`:

| Key | File |
|---|---|
| `darwin-aarch64` | `Librarium_X.Y.Z_aarch64.app.tar.gz` |
| `darwin-x86_64` | `Librarium_X.Y.Z_x64.app.tar.gz` |
| `windows-x86_64-nsis`, `windows-x86_64` | `Librarium_X.Y.Z_x64-setup.exe` |
| `windows-x86_64-msi` | `Librarium_X.Y.Z_x64_en-US.msi` |
| `linux-x86_64-appimage` | `Librarium_X.Y.Z_amd64.AppImage` |

MSI installs now update with the MSI, and NSIS installs with the NSIS installer. Before, every Windows install was sent the NSIS installer, so an MSI install ended up with a second, parallel NSIS install. The plain `windows-x86_64` key keeps older clients on the NSIS installer, as before. There is deliberately no plain `linux-x86_64` key: a `.deb` install would fall back to it and try to install an AppImage over a packaged app. `.deb` users update by installing the next `.deb`. The extra `downloads` field (direct installer links) is kept from the previous format.

The signatures are bound to the version (`version:X.Y.Z` in the signed trusted comment). Consider setting `plugins.updater.requireSignedVersion: true` in `tauri.conf.json`: the app then rejects any update whose signature isn't bound to the announced version, so a tampered `latest.json` can't pair a new version number with an older signed file. It is checked by the installed app, so it protects the versions that ship with it.

**Debug symbols.** Release binaries ship without debug info but keep their symbol tables. The line tables go to a `symbols-X.Y.Z-<target>` artifact on the workflow run: `librarium.dSYM` (macOS), `librarium.pdb` (Windows), `librarium.debug` (Linux, linked with `.gnu_debuglink`). Artifacts expire after 90 days, so download and keep them if you want to symbolicate crash reports for older releases. See the comments in `[profile.release]` in `src-tauri/Cargo.toml` for how each platform is handled.

## One-time setup

### Secrets

All secrets live in this repository: *Settings › Secrets and variables › Actions*. Secrets stay secret in a public repository: workflows triggered from forks (pull requests) never see them, and the release workflow runs only on tags, which only maintainers can push.

| Secret | What it is | Used by |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | The updater signing key: the content of `~/.tauri/librarium.key` (see below). | `sign` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The key's password (`tauri signer generate` asks for one). Empty only for a key without a password. | `sign` |
| `VITE_API_URL`, `VITE_API_KEY` | Backend API base URL and key, compiled into the frontend. Treat the key as public: anyone can read it from the app. | `build` |
| `VITE_UMAMI_WEBSITE_ID`, `VITE_UMAMI_URL` | Umami analytics website id and endpoint. | `build` |

To copy the key into the secret: `cat ~/.tauri/librarium.key`, copy the whole output. The value ends with `==`. Don't include the `%` some shells print after output without a trailing newline.

Recommended hardening, now that the repository is public (environments with protection rules are free for public repositories): create an environment named `release`, restrict it to `v*` tags (and add yourself as a required reviewer if you like), move the two signing secrets into it, and uncomment `environment: release` in the `sign` job.

Still to do: **pin the GitHub Actions to commit SHAs.** Both workflows reference third-party actions by version tag (marked `TODO(pin-actions)`); replace each with its full commit SHA and keep the tag as a comment, e.g. with [`pinact`](https://github.com/suzuki-shunsuke/pinact) or Dependabot's `github-actions` updates.

## Releasing from a fork

Everything above works from a fork once it has its own identity. Change, in one commit:

- **The updater:** generate your own key (`npx tauri signer generate -w ~/.tauri/librarium.key`, with a password), put its public half in `src-tauri/tauri.conf.json › plugins.updater.pubkey`, the two secrets in your repository, and point `plugins.updater.endpoints` to your repository's `releases/latest/download/latest.json`. Copies signed with someone else's key can't update to yours, and the other way round.
- **The app's identity:** `identifier` (the app's data, logs and caches folders are named after it; a different one keeps the two apps apart), `productName`, `bundle.publisher` and `bundle.copyright` in `tauri.conf.json`, and the author in `package.json` and `src-tauri/Cargo.toml`.
- **Your servers, if any:** the `VITE_*` secrets (see `.env.example`). Left unset, the app talks to no server of its own (no messages, error reports, feedback or statistics); `scripts/tauri.mjs` adds the hosts you set to the CSP.
- **PRIVACY.md**, which the app shows: who runs those servers, and how to reach you.

## The updater key

The key in `~/.tauri/librarium.key` (public half in `tauri.conf.json › plugins.updater.pubkey`, compiled into every installed copy) is what protects updates: the app installs an update only if its signature verifies against that public key. Since the builds aren't OS-signed (see below), it is the **only** protection. So:

- **Back it up** offline, e.g. in your password manager together with its password. If it is lost, existing installs can never auto-update again; users must reinstall by hand.
- **Never commit it**, and only paste it into the `TAURI_SIGNING_PRIVATE_KEY` secret.

### Adding a password (recommended, no impact on users)

A key without a password lets anyone who obtains the secret sign updates. If yours has none, put a password on the **same** key rather than generating a new one: the public key stays the same, so nothing changes for installed copies.

The key is a standard [minisign](https://jedisct1.github.io/minisign/) secret key, base64-encoded. With the minisign CLI (`brew install minisign`):

```bash
umask 077
base64 -d < ~/.tauri/librarium.key > /tmp/librarium.minisign.key
minisign -C -s /tmp/librarium.minisign.key        # current password: press Enter; then the new one
base64 < /tmp/librarium.minisign.key | tr -d '\n' > ~/.tauri/librarium-pw.key
rm /tmp/librarium.minisign.key

# Check the new file signs with the new password, and the signature matches the app's public key:
echo test > /tmp/t.txt
TAURI_SIGNING_PRIVATE_KEY_PASSWORD='<new password>' npx tauri signer sign -f ~/.tauri/librarium-pw.key --app-version 0.0.0 /tmp/t.txt
mkdir -p /tmp/sig && mv /tmp/t.txt.sig /tmp/sig/
node scripts/ci/minisign.mjs --version 0.0.0 --sig-dir /tmp/sig /tmp/t.txt   # must print "ok"
```

The last two commands prove it worked. Then store the content of `librarium-pw.key` in `TAURI_SIGNING_PRIVATE_KEY`, the password in `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, replace `~/.tauri/librarium.key` (and its backup) with the new file, and delete the old one.

### Rotating the key (only if it leaked or was lost)

Changing the key pair means installed copies (which trust the old public key) reject updates signed with the new one. If you still have the old key, do it in two releases:

1. Generate a new key **with a password**: `npx tauri signer generate -w ~/.tauri/librarium-new.key` (it asks for the password).
2. Put the new public key (`librarium-new.key.pub`) in `tauri.conf.json › plugins.updater.pubkey` and release as usual, still signing with the **old** key in the secrets. Installed copies accept this update, and from then on trust the new key.
3. Switch both signing secrets to the new key and password. Every later release is signed with it.

Users who skip the transition release must reinstall by hand. If the old key is lost or compromised, skip step 2: every user has to reinstall manually, so announce it.

## Builds are not OS-signed (yet)

There is no Apple Developer ID or Windows Authenticode certificate for now: macOS builds are ad-hoc signed, Windows builds are unsigned. Every release page explains the first launch (the template is `.github/release-notes-template.md`):

- **macOS:** after the first attempt to open it, *System Settings › Privacy & Security › Open Anyway*. macOS may ask again for folder and removable-volume access after an update, because the ad-hoc identity changes with every build.
- **Windows:** SmartScreen › *More info › Run anyway*.
- **Linux AppImage:** `chmod +x Librarium_X.Y.Z_amd64.AppImage`, then run it (needs FUSE 2: `libfuse2t64` on Ubuntu).

To revisit later: Apple Developer ID plus notarisation, and Windows Authenticode (e.g. Azure Trusted Signing).

## Linux packages

- **AppImage** updates itself through the in-app updater. **`.deb`** doesn't (see `latest.json` above): users install the next `.deb`.
- Built on Ubuntu 24.04, so they need glibc 2.39 or newer (Ubuntu 24.04, Debian 13, Fedora 40 and later).
- The `.deb` depends on `xdg-utils` (opening links and folders) and recommends `gstreamer1.0-plugins-good` (the app's sounds). The AppImage doesn't bundle GStreamer (`bundleMediaFramework: false`, it would add 15–35 MB), so sounds may not play there.
- **Kindle udev rule** (`src-tauri/linux/70-librarium-kindle.rules`): gives the logged-in user access to Amazon USB devices (vendor `1949`) for the experimental MTP support of 2024+ Kindles (on by default). The `.deb` installs it to `/usr/lib/udev/rules.d/`. AppImage users install it by hand; the file is attached to every release:

  ```bash
  sudo install -m 644 70-librarium-kindle.rules /etc/udev/rules.d/
  sudo udevadm control --reload-rules && sudo udevadm trigger
  ```

  then reconnect the Kindle.
- Before announcing Linux support, check on a real machine: mass-storage Kindle/Kobo detection, WebKitGTK rendering and performance, fonts, "reveal in file manager", and the Trash.

## Building locally

```bash
npm run build      # = tauri build: unsigned, for the current platform
```

Output goes to `src-tauri/target/release/bundle/`. Local builds are never signed with the updater key (`bundle.createUpdaterArtifacts` is `false`; CI creates and signs the updater artifacts itself), so no key or password is needed on your machine. The Rust toolchain version comes from `rust-toolchain.toml` (rustup installs it on first use), Node from `.nvmrc` (Node ≥ 20.19 is required).

## How the in-app updater works

At startup the app fetches `https://github.com/myeti/librarium/releases/latest/download/latest.json` (`plugins.updater.endpoints`). Drafts are invisible there, so nothing reaches users before the `publish` job has verified the release. If the version is newer, the app shows the release notes and offers to install. It downloads the file for its platform and bundle type, checks the signature against the public key compiled into it, then installs and restarts. If the endpoint is unreachable, the check fails silently.

## Troubleshooting

**`check` fails: version mismatch.** The tag doesn't match `package.json`/`Cargo.toml`/`Cargo.lock`. Run `npm run bump`, commit, delete the bad tag (`git push origin :refs/tags/vX.Y.Z`, `git tag -d vX.Y.Z`) and release again.

**`check` fails: lightweight tag or empty notes.** Tag with `npm run release`, which creates an annotated tag whose message is the notes.

**`build` fails at `npm ci` or with `--locked`.** A lockfile doesn't match its manifest: run `npm install` (or a local `cargo build`) to update it, commit, delete the tag as above and run `npm run release` again.

**`sign` fails with "incorrect updater private key password".** `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` doesn't match the key in `TAURI_SIGNING_PRIVATE_KEY`.

**`sign` fails: a signature doesn't verify.** The key in the secret isn't the one whose public key is in `tauri.conf.json`. Don't publish: fix the secret (see *The updater key*).

**`publish` fails with 401/403.** The workflow's token may not write releases: the job asks for `contents: write`, which a stricter setting can refuse (an organisation policy, or *Settings › Actions › General › Workflow permissions* if it ever moves to an organisation). Allow it, then *Re-run failed jobs*: an existing draft for the tag is reused.

**`publish` fails at "Verify what the draft serves" with a diff.** The draft holds a file this run didn't build (left over from an earlier attempt). Delete that asset from the draft (this repository's *Releases* page), then re-run the job.

**In-app updater never shows an update.** Check that `latest.json` exists at the URL above and lists your platform. The `.deb` build never auto-updates, by design.
