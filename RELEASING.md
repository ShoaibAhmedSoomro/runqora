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
