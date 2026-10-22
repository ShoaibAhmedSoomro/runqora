<#
  Theme and accent in the REAL window: does it follow Windows, and does the window frame follow the setting?
  Usage: powershell -File e2e\appearance.ps1 [-Root <install folder>]      Exit code = number of failed checks.
  Changes general.theme / accent settings in that install and puts them back.
#>
param([string]$Root = "$env:TEMP\rt-smoke", [string]$App = "C:\rt-target\debug\runqora.exe", [string]$Rq = "C:\rt-target\debug\rq.exe", [int]$Port = 9334)
$env:RUNQORA_DIR = $Root; $env:CDP_PORT = $Port
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class DwmA { [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out int v, int s); }' -ErrorAction SilentlyContinue
$fail = @(); $n = 0
function Check($id, $what, [bool]$ok, $got = "") { $script:n++; if ($ok) { Write-Host "  pass  $id  $what" } else { $script:fail += "$id $what ($got)"; Write-Host "  FAIL  $id  $what  ($got)" -ForegroundColor Red } }
function Probe($shot) { node (Join-Path $PSScriptRoot "probe.mjs") $shot | ConvertFrom-Json }
function FrameDark($p) { $v = 0; [DwmA]::DwmGetWindowAttribute($p.MainWindowHandle, 20, [ref]$v, 4) | Out-Null; $v -eq 1 }   # USE_IMMERSIVE_DARK_MODE
function Lum($hex) { $c = 1, 3, 5 | % { [Convert]::ToInt32($hex.Substring($_, 2), 16) / 255 } | % { if ($_ -le 0.03928) { $_ / 12.92 } else { [Math]::Pow(($_ + 0.055) / 1.055, 2.4) } }; 0.2126 * $c[0] + 0.7152 * $c[1] + 0.0722 * $c[2] }
function Contrast($a, $b) { $x = Lum $a; $y = Lum $b; if ($x -lt $y) { $x, $y = $y, $x }; ($x + 0.05) / ($y + 0.05) }
function ToHex($css) { if ($css -match '(\d+),\s*(\d+),\s*(\d+)') { '#{0:X2}{1:X2}{2:X2}' -f [int]$matches[1], [int]$matches[2], [int]$matches[3] } else { $css } }

Get-Process runqora -ErrorAction SilentlyContinue | Stop-Process -Force
$settings = "$Root\settings.json"; $saved = if (Test-Path $settings) { Get-Content $settings -Raw } else { $null }
& $Rq config reset general theme | Out-Null; & $Rq config reset general accent_mode | Out-Null
$p = Start-Process $App -PassThru
$up = $false; for ($i = 0; $i -lt 40 -and -not $up; $i++) { Start-Sleep -Milliseconds 500; try { $up = (Invoke-WebRequest "http://127.0.0.1:$Port/json/version" -UseBasicParsing -TimeoutSec 2).StatusCode -eq 200 } catch {} }
if (-not $up) { Write-Host "window did not start" -ForegroundColor Red; exit 99 }
Start-Sleep 6

Write-Host "`nfollowing Windows"
$osDark = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize" -ErrorAction SilentlyContinue).AppsUseLightTheme -eq 0
$reg = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Accent" -ErrorAction SilentlyContinue).AccentColorMenu
$winAccent = if ($null -ne $reg) { '#{0:X2}{1:X2}{2:X2}' -f ($reg -band 0xFF), (($reg -shr 8) -band 0xFF), (($reg -shr 16) -band 0xFF) } else { $null }
$s = Probe "appearance-system"
Check "APP-1" "no data-theme attribute: the page follows prefers-color-scheme" ($null -eq $s.dataTheme)
Check "APP-2" "the page's idea of dark equals Windows' app mode" ($s.osDark -eq $osDark) "page $($s.osDark), Windows $osDark"
Check "APP-3" "the frame (title bar / Mica) matches Windows" ((FrameDark $p) -eq $osDark)
Check "APP-4" "the app reads the same accent colour Windows stores" ($s.appearance.system_accent -eq $winAccent) "app $($s.appearance.system_accent), registry $winAccent"
Check "APP-5" "the accent button's label is readable on it (4.5:1)" ((Contrast (ToHex $s.accentButton.background) (ToHex $s.accentButton.label)) -ge 4.5) "$($s.accentButton.background) / $($s.accentButton.label)"
Check "APP-6" "accent used as text is readable on a card in this theme" ((Contrast $s.accent.text $(if ($osDark) { "#2B2B2B" } else { "#FFFFFF" })) -ge 4.5) $s.accent.text

Write-Host "`nchoosing a theme"
foreach ($t in "light", "dark") {
  & $Rq config set general theme $t | Out-Null; Start-Sleep 4
  $r = Probe "appearance-$t"
  Check "APP-7$t" "theme $t : data-theme is set, page background changes, and the window frame follows" (($r.dataTheme -eq $t) -and ((FrameDark $p) -eq ($t -eq "dark"))) "attr $($r.dataTheme), frame dark $(FrameDark $p)"
  Check "APP-8$t" "theme $t : accent text stays readable in that theme" ((Contrast $r.accent.text $(if ($t -eq "dark") { "#2B2B2B" } else { "#FFFFFF" })) -ge 4.5) $r.accent.text
}
& $Rq config set general theme system | Out-Null; Start-Sleep 4
$r = Probe "appearance-back"
Check "APP-9" "back to Match Windows: attribute removed and the frame returns to Windows' mode" (($null -eq $r.dataTheme) -and ((FrameDark $p) -eq $osDark))

Write-Host "`nchoosing an accent"
& $Rq config set general accent_mode custom | Out-Null
foreach ($c in "#E81123", "#FFEB3B", "#000000") {
  & $Rq config set general accent_color $c | Out-Null; Start-Sleep 4
  $r = Probe "appearance-accent"
  $bg = ToHex $r.accentButton.background
  Check "APP-10" "custom $c : buttons use a readable version of it (label 4.5:1)" ((Contrast $bg (ToHex $r.accentButton.label)) -ge 4.5) "$bg / $($r.accentButton.label)"
}
& $Rq config set general accent_color "#E81123" | Out-Null; Start-Sleep 4; $r = Probe "appearance-red"
Check "APP-11" "a red accent really is red (hue kept, not forced back to blue)" ((ToHex $r.accentButton.background) -match '^#[C-F]') (ToHex $r.accentButton.background)
& $Rq config set general accent_mode system | Out-Null; Start-Sleep 4; $r = Probe ""
Check "APP-12" "back to Match Windows: buttons use Windows' accent again" ($r.appearance.accent_mode -eq "system") $r.appearance.accent_mode

Stop-Process $p -Force
if ($saved) { Set-Content $settings $saved -NoNewline -Encoding ascii } else { Remove-Item $settings -ErrorAction SilentlyContinue }
Write-Host ("`npassed {0}, failed {1}" -f ($n - $fail.Count), $fail.Count)
$fail | % { Write-Host "  FAILED: $_" -ForegroundColor Red }
exit $fail.Count
