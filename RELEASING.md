# Releasing a new version

Runqora updates itself from this repository's GitHub releases. Publishing a release is all it takes for every
installed copy to offer the update.

## Publish

```powershell
powershell -File scripts\release.ps1 -Version 0.2.1 -Notes "What changed, in a sentence or two."
```

The script bumps the version, builds the installer, names it `Runqora-Setup-x64.exe`, writes `SHA256.txt`, commits,
tags `v0.2.1`, pushes and creates the release. Commit your work first; the script refuses to run with uncommitted
changes so the release commit holds only the version bump.

## Linux and macOS

The Windows installer is built on the machine that runs `release.ps1`. Linux and macOS packages are built by GitHub
Actions and attached to the same release:

1. Commit and push, then publish the Windows release as above (this creates the tag `v0.2.1` and the release).
2. Run **Actions > Release Linux and macOS** with that tag. It builds `Runqora-linux-x86_64.deb`/`.AppImage`,
   `Runqora-linux-aarch64.deb`/`.AppImage` and `Runqora-macos-aarch64.dmg` and uploads them to the release.
3. Apache, PHP and MariaDB for these systems are not rebuilt per release. When a version of one of them changes, run
   **Actions > Stack** with the next number (it publishes release `stack-<n>`), copy the `stack.json` from that release to
   `app/src-tauri/stack.json`, run `python scripts/gen-unix-manifest.py`, commit the new `manifest.unix.json`, then release.

**Actions > Unix end-to-end** installs the stack on all three systems and runs `e2e/unix.sh` (56 checks); run it before a
release. Installed copies on Linux and macOS find the update the same way and save the new `.AppImage` or `.dmg` to
`~/Downloads` (they never replace themselves while running).

## What users see

- **Installed copies** check the latest release when they start and then every 30 minutes. A newer version appears as a
  notice, a badge on **Updates**, and a **Runqora** card with the release notes. **Update** downloads the installer
  from the release, checks it against the SHA-256 GitHub shows for the file, and opens it; the app closes so it can be
  replaced. Servers keep running and `C:\Runqora` is untouched.
- **The website** (`docs/`) has a Download button that always points at the latest release, and shows its version.
- `rq app-update` does the same check from a terminal (`--yes` to install).

## Rules the updater relies on

- The release tag is `v<version>` and the version is higher than the one in `Cargo.toml` of the running build.
- The release carries an asset named exactly `Runqora-Setup-x64.exe`.
- Only releases of this repository are ever offered: the app refuses any other download address.
- Draft and pre-release entries are ignored (GitHub's "latest" skips them).

## Checking before you publish

```powershell
cargo test --lib                       # in app\src-tauri
powershell -File e2e\run.ps1 -Network  # reads every publisher; downloads nothing
powershell -File e2e\ui.ps1            # layout audit of the real window
```
