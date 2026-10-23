<#
  Publish a new Runqora version in one step.

    powershell -File scripts\release.ps1 -Version 0.2.1 -Notes "What changed, in a sentence or two."

  What it does, in order (it stops at the first problem and changes nothing on GitHub until the installer exists):
    1. sets the version in Cargo.toml and tauri.conf.json
    2. builds the release programs and the NSIS installer
    3. names the installer Runqora-Setup-x64.exe and writes SHA256.txt next to it
    4. commits the version change, tags v<Version>, pushes
    5. creates the GitHub release with both files

  Every running copy of Runqora then offers the update on its Updates page (it reads this release), and the website's
  download button, which points at the latest release, serves the new installer.
  Needs: Rust, Node (for npx), and the GitHub CLI signed in as the repository owner.
#>
param(
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
  [string]$Notes = "",
  [string]$Repo = "ShoaibAhmedSoomro/runqora"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$tauri = Join-Path $root "app\src-tauri"
Set-Location $root

if (git status --porcelain) { throw "Commit or stash your changes first: the release commit must contain only the version bump." }
$old = (Select-String -Path "$tauri\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
if ([version]$Version -lt [version]$old) { throw "Version $Version is older than $old." }  # equal is fine: the bump may already be committed
$ErrorActionPreference = "Continue"  # gh prints "release not found" to stderr, which Windows PowerShell 5.1 would treat as fatal
gh release view "v$Version" --repo $Repo *> $null
$exists = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = "Stop"
if ($exists) { throw "Release v$Version already exists." }

# 1. version
# no byte-order mark: Windows PowerShell 5.1's "-Encoding utf8" adds one, and Tauri cannot read JSON that starts with it
$utf8 = New-Object System.Text.UTF8Encoding($false)
$cargo = [IO.File]::ReadAllText("$tauri\Cargo.toml")
[IO.File]::WriteAllText("$tauri\Cargo.toml", ($cargo -replace '(?m)^version = ".+"', "version = `"$Version`""), $utf8)
$conf = [IO.File]::ReadAllText("$tauri\tauri.conf.json")
[IO.File]::WriteAllText("$tauri\tauri.conf.json", ($conf -replace '"version":\s*"[^"]+"', "`"version`": `"$Version`""), $utf8)

# 2. build
Push-Location $tauri
try {
  cargo build --release
  if ($LASTEXITCODE) { throw "cargo build failed" }
  npx --yes "@tauri-apps/cli@2.12.4" build --bundles nsis
  if ($LASTEXITCODE) { throw "the installer build failed" }
} finally { Pop-Location }

# 3. files
$target = (Select-String -Path "$tauri\.cargo\config.toml" -Pattern 'target-dir\s*=\s*"(.+)"' -ErrorAction SilentlyContinue).Matches.Groups[1].Value
if (-not $target) { $target = "$tauri\target" }
$built = Get-ChildItem "$target\release\bundle\nsis\Runqora_${Version}_x64-setup.exe" -ErrorAction Stop
$out = Join-Path ([IO.Path]::GetTempPath()) "runqora-release-$Version"
New-Item -ItemType Directory -Force $out | Out-Null
$exe = Join-Path $out "Runqora-Setup-x64.exe"
Copy-Item $built.FullName $exe -Force
$hash = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
"$hash *Runqora-Setup-x64.exe" | Set-Content (Join-Path $out "SHA256.txt") -Encoding ascii
Write-Host "installer: $exe  sha256 $hash"

# 4. commit, tag, push
git add "$tauri\Cargo.toml" "$tauri\Cargo.lock" "$tauri\tauri.conf.json"
if (git status --porcelain) { git commit -m "chore: release $Version" }
git tag "v$Version"
git push origin HEAD "v$Version"

# 5. release
if (-not $Notes) { $Notes = "Runqora $Version." }
$Notes += "`n`nInstall or update: run `Runqora-Setup-x64.exe`. SHA-256 of the installer is in `SHA256.txt`."
gh release create "v$Version" $exe (Join-Path $out "SHA256.txt") --repo $Repo --title "Runqora $Version" --notes $Notes
Write-Host "Published: https://github.com/$Repo/releases/tag/v$Version"
