<#
  Layout audit of the REAL Runqora window (no mock): launches it with a debugging port, audits every page, tab and dialog
  at 560, 700, 840, 1040 and 1400 px for overlapping text, content sticking out, clipped text, small hit targets,
  unlabelled controls and missing logos, saves screenshots to %TEMP%\runqora-shots, then closes the app.
  Usage:  powershell -File e2e\ui.ps1 [-Root <install folder>] [-App <runqora.exe>]     Exit code 0 = clean.
#>
param([string]$Root = "$env:TEMP\rt-smoke", [string]$App = "C:\rt-target\debug\runqora.exe", [int]$Port = 9333)
$env:RUNQORA_DIR = $Root
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
$env:CDP_PORT = $Port
Get-Process runqora -ErrorAction SilentlyContinue | Stop-Process -Force
$p = Start-Process $App -PassThru
$ready = $false; for ($i = 0; $i -lt 40 -and -not $ready; $i++) { Start-Sleep -Milliseconds 500; try { $ready = (Invoke-WebRequest "http://127.0.0.1:$Port/json/version" -UseBasicParsing -TimeoutSec 2).StatusCode -eq 200 } catch {} }
if (-not $ready) { Stop-Process $p -Force; Write-Host "the window never opened its debugging port" -ForegroundColor Red; exit 99 }
Start-Sleep 5
$json = node (Join-Path $PSScriptRoot "cdp.mjs") --audit | Out-String
$code = $LASTEXITCODE
Stop-Process $p -Force -ErrorAction SilentlyContinue
$r = $json | ConvertFrom-Json
foreach ($w in $r.audit.PSObject.Properties) { Write-Host ("{0,5} px  {1}" -f $w.Name, $(if ($w.Value.Count) { "$($w.Value.Count) page(s) with findings" } else { "clean" })) ; foreach ($s in $w.Value) { Write-Host "       $($s.step)"; $s.issues | select -First 5 | % { Write-Host "         $_" } } }
Write-Host "screenshots: $env:TEMP\runqora-shots"
exit $code
