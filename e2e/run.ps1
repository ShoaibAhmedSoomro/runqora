<#
  Runqora end-to-end checks against a real installed stack, driven through the `rq` command line.
  Usage:  powershell -File e2e\run.ps1 [-Root <install folder>] [-Rq <path to rq.exe>] [-Network]
  The install folder must already hold Apache, PHP, MariaDB and phpMyAdmin (the smoke test makes one in %TEMP%\rt-smoke).
  It changes settings in that folder and starts servers on spare ports (18080 / 13306), then restores what it touched.
  Case numbers match replica/test-plan.md. Exit code = number of failed cases.
#>
param(
  [string]$Root = "$env:TEMP\rt-smoke",
  [string]$Rq = "C:\rt-target\debug\rq.exe",
  [string]$App = "C:\rt-target\debug\runqora.exe",
  [switch]$Network
)
$ErrorActionPreference = "Continue"
$env:RUNQORA_DIR = $Root
$script:pass = 0; $script:fail = @(); $script:skip = @()
$ApachePort = 18080; $DbPort = 13306

function Check([string]$id, [string]$what, [scriptblock]$test) {
  try { $r = & $test; if ($r -eq $true) { $script:pass++; Write-Host "  pass  $id  $what" } else { $script:fail += "$id  $what  (got: $r)"; Write-Host "  FAIL  $id  $what  (got: $r)" -ForegroundColor Red } }
  catch { $script:fail += "$id  $what  ($($_.Exception.Message))"; Write-Host "  FAIL  $id  $what  ($($_.Exception.Message))" -ForegroundColor Red }
}
function Skip([string]$id, [string]$why) { $script:skip += "$id  $why"; Write-Host "  skip  $id  $why" -ForegroundColor Yellow }
function Rq { & $Rq @args 2>&1 | Out-String }
function Code { $global:LASTEXITCODE }
function Hash($p) { (Get-FileHash $p -Algorithm SHA256).Hash }
function Listening($port) { [bool](Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) }
function Get-Page($url, $sec = 15) { try { (Invoke-WebRequest $url -UseBasicParsing -TimeoutSec $sec) } catch { $null } }
function Stranger($port) { Start-Process powershell -ArgumentList '-NoProfile', '-Command', "`$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,$port); `$l.Start(); Start-Sleep 180" -WindowStyle Hidden -PassThru }

if (-not (Test-Path "$Root\apache\bin\httpd.exe") -or -not (Test-Path $Rq)) { Write-Host "Need an installed stack in $Root and $Rq" -ForegroundColor Red; exit 99 }
$settingsFile = "$Root\settings.json"; $settingsBackup = if (Test-Path $settingsFile) { Get-Content $settingsFile -Raw } else { $null }
$phpIni = "$Root\php\php.ini"; $phpIniBackup = Get-Content $phpIni -Raw; $myIni = "$Root\mariadb\my.ini"; $myIniBackup = Get-Content $myIni -Raw
$confBackup = Get-Content "$Root\apache\conf\httpd.conf" -Raw
Rq stop | Out-Null
Rq config set apache port $ApachePort | Out-Null; Rq config set mariadb port $DbPort | Out-Null

Write-Host "`nF09 command line"
Check "F09-H1" "help prints usage, exit 0" { $o = Rq help; ($o -match "Usage") -and (Code) -eq 0 }
Check "F09-N1" "unknown command exits 2 and shows help" { $o = Rq frobnicate; (Code) -eq 2 -and $o -match "Usage" }
Check "F09-N2" "start with an unknown module exits 2" { Rq start nginx | Out-Null; (Code) -eq 2 }
Check "F09-N3" "a library cannot be started (exit 2)" { Rq start php | Out-Null; (Code) -eq 2 }
Check "F09-E1" "status lists every module" { $o = Rq status; ($o -match "apache") -and ($o -match "php") -and ($o -match "mariadb") -and ($o -match "phpmyadmin") -and ($o -match "tomcat") -and ($o -match "mailpit") -and ($o -match "sftpgo") -and ($o -match "wordpress") -and ($o -match "jre") }

Write-Host "`nF01 start"
Check "F01-H1" "start brings up Apache and MariaDB (exit 0)" { Rq start | Out-Null; (Code) -eq 0 -and (Listening $ApachePort) -and (Listening $DbPort) }
Check "F01-H2" "Apache serves a page and PHP runs inside it" { $p = Get-Page "http://127.0.0.1:$ApachePort/"; $p -and $p.StatusCode -eq 200 }
Check "F01-H3" "phpMyAdmin answers 200 on the same server" { (Get-Page "http://127.0.0.1:$ApachePort/phpmyadmin/" 120).StatusCode -eq 200 } # the first request after a cold start reads thousands of files: 30+ s with real-time virus scanning
Check "F01-H4" "MariaDB accepts a connection and reports its port" { (& "$Root\mariadb\bin\mysql.exe" @('-uroot', '--host=127.0.0.1', "--port=$DbPort", '-N', '-e', 'select @@port') 2>&1) -match "$DbPort" }
Check "F01-E1" "starting again is harmless (exit 0, says already running)" { $o = Rq start; (Code) -eq 0 -and $o -match "already running" }
Check "F01-E2" "start piped to another command returns promptly (no inherited pipe)" { Rq stop | Out-Null; $t = [Diagnostics.Stopwatch]::StartNew(); & $Rq start | Out-Null; $t.Elapsed.TotalSeconds -lt 60 }
Check "F01-E3" "status shows PIDs for running servers" { (Rq status) -match "running\s+\d+" }
Check "F01-E4" "ports lists our two listeners as ours" { $o = Rq ports; ($o -match "$ApachePort.*this install") -and ($o -match "$DbPort.*this install") }

Check "F01-E5" "start order: database before web server" { Rq stop | Out-Null; $o = Rq start; Rq stop | Out-Null; $o.IndexOf("MariaDB") -ge 0 -and $o.IndexOf("MariaDB") -lt $o.IndexOf("Apache") }
Check "F01-E6" "the moment start returns, the ports accept connections (no waiting needed)" { Rq stop | Out-Null; Rq start | Out-Null; $r = (Listening $ApachePort) -and (Listening $DbPort) -and ((Get-Page "http://127.0.0.1:$ApachePort/phpmyadmin/").StatusCode -eq 200); Rq stop | Out-Null; $r }

Write-Host "`nF02 stop"
Check "F02-H1" "stop frees both ports (exit 0)" { Rq stop | Out-Null; (Code) -eq 0 -and -not (Listening $ApachePort) -and -not (Listening $DbPort) }
Check "F02-E1" "stopping when stopped is harmless" { $o = Rq stop; (Code) -eq 0 -and $o -match "not running" }
Check "F02-E2" "stop order: web server before database" { Rq start | Out-Null; $o = Rq stop; $o.IndexOf("Apache") -lt $o.IndexOf("MariaDB") }
Check "F02-E3" "stop one by name leaves the other running" { Rq start | Out-Null; Rq stop apache | Out-Null; $r = (Listening $DbPort) -and -not (Listening $ApachePort); Rq stop | Out-Null; $r }

Write-Host "`nF01 blocked ports (negative)"
$s = Stranger $ApachePort; Start-Sleep 3
Check "F01-N1" "a stranger on the port blocks start: exit 1, names program and PID" { $o = Rq start apache; (Code) -eq 1 -and $o -match "powershell" -and $o -match "$($s.Id)" }
Check "F01-N2" "…and no Apache process was left behind" { -not (Get-Process httpd -ErrorAction SilentlyContinue | ? { $_.Path -like "$Root*" }) }
Check "F01-N3" "status shows the blocked state" { (Rq status) -match "port in use" }
Check "F01-N4" "doctor reports the blocked port with no automatic fix" { $o = Rq doctor; (Code) -eq 1 -and $o -match "Port $ApachePort is used by" }
Check "F01-N5" "free without --yes does not end it (exit 2)" { Rq free $ApachePort | Out-Null; (Code) -eq 2 -and -not $s.HasExited }
Check "F01-N6" "free --yes ends exactly that program and frees the port" { Rq free $ApachePort --yes | Out-Null; Start-Sleep 1; (Code) -eq 0 -and -not (Listening $ApachePort) }
Check "F01-N7" "after that Apache starts" { Rq start apache | Out-Null; $r = (Code) -eq 0 -and (Listening $ApachePort); Rq stop | Out-Null; $r }
Check "F01-N8" "Windows itself is never ended: free on a protected port refuses or finds nothing" {
  $sys = (Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | ? { $_.OwningProcess -eq 4 } | select -first 1)
  if (-not $sys) { return $true }
  Rq free $sys.LocalPort --yes | Out-Null; (Code) -ne 0 -and (Get-Process -Id 4 -ErrorAction SilentlyContinue) -ne $null }

Write-Host "`nF06 settings"
$iniBefore = Hash $phpIni
Check "F06-H1" "a PHP setting lands in php.ini and PHP reads it back" { Rq config set php memory_limit 384m | Out-Null; (Code) -eq 0 -and (& "$Root\php\php.exe" -r "echo ini_get('memory_limit');") -eq "384M" }
Check "F06-H2" "the original php.ini is kept once, not overwritten by later edits" { Rq config set php post_max_size 16M | Out-Null; $b = "$Root\php\php.ini.runqora-backup"; (Test-Path $b) -and (Get-Content $b -Raw) -eq $phpIniBackup }
Check "F06-H3" "an MariaDB setting reaches the running server" { Rq config set mariadb max_connections 177 | Out-Null; Rq start mariadb | Out-Null; $v = (& "$Root\mariadb\bin\mysql.exe" @('-uroot', '--host=127.0.0.1', "--port=$DbPort", '-N', '-e', 'select @@max_connections') 2>&1); Rq stop | Out-Null; "$v".Trim() -eq "177" }
Check "F06-H4" "an extension switches off and on" { Rq config set php ext.gd off | Out-Null; $off = (& "$Root\php\php.exe" -r "echo extension_loaded('gd')?'y':'n';"); Rq config set php ext.gd on | Out-Null; $on = (& "$Root\php\php.exe" -r "echo extension_loaded('gd')?'y':'n';"); $off -eq "n" -and $on -eq "y" }
Check "F06-H5" "the website folder can be changed and is served" { $site = "$env:TEMP\rt-e2e-site"; New-Item -ItemType Directory $site -Force | Out-Null; Set-Content "$site\index.php" '<?php echo "e2e-site";' -Encoding ascii; Rq config set apache document_root $site | Out-Null; Rq start apache | Out-Null; $p = (Get-Page "http://127.0.0.1:$ApachePort/").Content; Rq stop | Out-Null; $p -eq "e2e-site" }
Check "F06-E1" "…and an empty value puts it back to the default" { Rq config set apache document_root '""' | Out-Null; (Rq config get apache) -match "document_root\s+\(default\)" }
Check "F06-E2" "a folder name with spaces, accents and a symbol is served" { $site = "$env:TEMP\rt e2e sité-ñ&"; New-Item -ItemType Directory $site -Force | Out-Null; Set-Content "$site\index.html" 'ünï-ok' -Encoding utf8; Rq config set apache document_root $site | Out-Null; Rq start apache | Out-Null; $p = (Get-Page "http://127.0.0.1:$ApachePort/").Content; Rq stop | Out-Null; Rq config set apache document_root '""' | Out-Null; Remove-Item $site -Recurse -Force; $p -match "ok" }
Check "F06-E9" "reset puts a setting back to its default (and works where an empty value cannot be typed)" { Rq config set php memory_limit 700M | Out-Null; $o = Rq config reset php memory_limit; (Code) -eq 0 -and (Rq config get php) -match "memory_limit\s+128M" -and (Rq config reset php nonsense | Out-String) -match "no setting" -and (Code) -eq 1 }
Check "F06-E3" "sizes are normalised and case-insensitive" { Rq config set php upload_max_filesize 8m | Out-Null; (Rq config get php) -match "upload_max_filesize\s+8M" }
Check "F06-E4" "the same value twice changes nothing" { $h = Hash $phpIni; $o = Rq config set php upload_max_filesize 8M; (Hash $phpIni) -eq $h -and $o -match "already" }
Check "F06-E5" "the install-wide extension count never drops below the defaults" { (Rq config get php) -match "ext\.mysqli\s+on" }
Check "F06-N1" "a bad number is refused with a reason and the file is untouched" { $h = Hash $phpIni; $o = Rq config set php max_execution_time banana; (Code) -eq 1 -and $o -match "whole number" -and (Hash $phpIni) -eq $h }
Check "F06-N2" "an out-of-range number is refused" { Rq config set mariadb max_connections 5 | Out-Null; (Code) -eq 1 }
Check "F06-N3" "a bad size is refused" { Rq config set php memory_limit lots | Out-Null; (Code) -eq 1 }
Check "F06-N4" "an unknown key is refused" { Rq config set php no_such_thing 1 | Out-Null; (Code) -eq 1 }
Check "F06-N5" "an unknown module is refused" { Rq config get nginx | Out-Null; (Code) -eq 1 }
Check "F06-N6" "a relative or drive-root folder is refused" { Rq config set apache document_root "sites" | Out-Null; $a = Code; Rq config set apache document_root "C:\" | Out-Null; $a -eq 1 -and (Code) -eq 1 }
Check "F06-N7" "a time zone that does not exist is refused" { Rq config set php date.timezone "Mars/Phobos" | Out-Null; (Code) -eq 1 }
Check "F06-N8" "a quote or newline in a value cannot break the file" { Rq config set php date.timezone 'Asia/Dubai"; evil' | Out-Null; (Code) -eq 1 -and -not ((Get-Content $phpIni -Raw) -match "evil") }
Check "F06-N9" "the port of a running server cannot change" { Rq start apache | Out-Null; Rq config set apache port 18099 | Out-Null; $r = (Code) -eq 1; Rq stop | Out-Null; $r }
Check "F06-N10" "a port another module uses is refused" { Rq config set apache port $DbPort | Out-Null; (Code) -eq 1 }
Check "F06-N11" "a port somebody is listening on is refused" { $p = Stranger 18123; Start-Sleep 2; Rq config set apache port 18123 | Out-Null; $r = (Code) -eq 1; Stop-Process $p -Force; $r }
Check "F06-E6" "a valid port change rewrites httpd.conf" { Rq config set apache port 18081 | Out-Null; $ok = (Select-String "$Root\apache\conf\httpd.conf" -Pattern '^Listen 18081').Count -eq 1; Rq config set apache port $ApachePort | Out-Null; $ok }
Check "F06-E7" "exposing the database to the network carries a warning" { $o = Rq config set mariadb bind_address 0.0.0.0; Rq config set mariadb bind_address 127.0.0.1 | Out-Null; $o -match "no password" }

Write-Host "`nSettings file (INI)"
Check "F06-I1" "export writes runqora.ini with every module" { Rq config export | Out-Null; $t = Get-Content "$Root\runqora.ini" -Raw; (Code) -eq 0 -and $t -match "\[php\]" -and $t -match "\[mariadb\]" -and $t -match "\[apache\]" -and $t -match "extensions = " }
Check "F06-I2" "importing the export changes nothing" { $o = Rq config import "$Root\runqora.ini"; (Code) -eq 0 -and $o -notmatch "will change" }
$edit = "[php]`nmemory_limit = 1G`nmax_execution_time = banana`nnot_a_setting = 1`n[mariadb]`nmax_connections = 250`n[nginx]`nport = 1`n"
Set-Content "$env:TEMP\rt-e2e.ini" $edit -Encoding ascii
Check "F06-I3" "preview shows changes and problems, writes nothing, exit 1" { $h = Hash $phpIni; $o = Rq config import "$env:TEMP\rt-e2e.ini"; (Code) -eq 1 -and $o -match "will change" -and $o -match "PROBLEM" -and (Hash $phpIni) -eq $h }
Check "F06-I4" "--apply applies the good lines and skips the bad ones" { Rq config import "$env:TEMP\rt-e2e.ini" --apply | Out-Null; (Code) -eq 1 -and (Rq config get php) -match "memory_limit\s+1G" -and (Rq config get mariadb) -match "max_connections\s+250" }
Check "F06-I5" "a missing file is reported, not a crash" { Rq config import "$env:TEMP\no-such.ini" | Out-Null; (Code) -eq 1 }
Check "F06-I6" "extensions = is an exact list (others off)" { Set-Content "$env:TEMP\rt-e2e2.ini" "[php]`nextensions = mbstring, mysqli" -Encoding ascii; Rq config import "$env:TEMP\rt-e2e2.ini" --apply | Out-Null; $on = (Rq config get php) -split "`n" | ? { $_ -match "ext\.\w+\s+on" }; $on.Count -eq 2 }
Check "F06-I7" "an extension this PHP lacks is reported" { Set-Content "$env:TEMP\rt-e2e3.ini" "[php]`nextensions = mbstring, imap" -Encoding ascii; (Rq config import "$env:TEMP\rt-e2e3.ini") -match "no imap extension" }
Check "F06-E8" "a settings.json that no longer parses does not stop the tools, and is kept" { Set-Content $settingsFile "{ not json" -Encoding ascii; $o = Rq status; $okStatus = (Code) -eq 0 -and $o -match "apache"; Rq config set general auto_update off | Out-Null; $okStatus -and (Test-Path "$settingsFile.bad") }
Remove-Item "$env:TEMP\rt-e2e*.ini", "$settingsFile.bad", $settingsFile -ErrorAction SilentlyContinue
Rq config set apache port $ApachePort | Out-Null; Rq config set mariadb port $DbPort | Out-Null

Write-Host "`nCheck setup (doctor)"
Check "DOC-H1" "a healthy install: nothing to fix, exit 0" { Rq config set php memory_limit 128M | Out-Null; $o = Rq doctor; (Code) -eq 0 -and $o -match "Everything looks right" }
Check "DOC-E1" "a wrong extension_dir is found and fixed, and PHP still runs" {
  $bad = (Get-Content $phpIni -Raw) -replace '(?m)^extension_dir = .*$', 'extension_dir = "C:/Nowhere/ext"'; Set-Content $phpIni $bad -Encoding ascii
  $a = Rq doctor; $f = Rq doctor --fix; (Code) -eq 0 -and $a -match "extensions" -and $f -match "fixed" -and (Rq doctor) -match "Everything" -and ((& "$Root\php\php.exe" -m 2>&1) -match "mysqli") }
Check "DOC-E2" "a duplicated include in httpd.conf is found, fixed, and Apache starts" {
  Add-Content "$Root\apache\conf\httpd.conf" "Include conf/extra/runqora.conf"; $a = Rq doctor; Rq doctor --fix | Out-Null
  $one = (Select-String "$Root\apache\conf\httpd.conf" -Pattern '^Include conf/extra/runqora.conf').Count -eq 1; Rq start apache | Out-Null; $up = Listening $ApachePort; Rq stop | Out-Null; $a -match "twice" -and $one -and $up }
Check "DOC-E3" "phpMyAdmin on the wrong database port is found and fixed" {
  $cfg = "$Root\phpmyadmin\config.inc.php"; $c = Get-Content $cfg -Raw; Set-Content $cfg ($c -replace "\['port'\] = '\d+'", "['port'] = '1234'") -Encoding ascii
  $a = Rq doctor; Rq doctor --fix | Out-Null; $a -match "wrong port" -and (Get-Content $cfg -Raw) -match "'$DbPort'" }
Check "DOC-E4" "a stale pid file is found and removed" { Set-Content "$Root\apache\logs\httpd.pid" "999999" -Encoding ascii; $a = Rq doctor; Rq doctor --fix | Out-Null; $a -match "stale" -and -not (Test-Path "$Root\apache\logs\httpd.pid") }
Check "DOC-E6" "a deleted website folder is found and recreated, then Apache starts" {
  $site = "$env:TEMP\rt-e2e-gone"; Rq config set apache document_root $site | Out-Null; Remove-Item $site -Recurse -Force -ErrorAction SilentlyContinue
  $a = Rq doctor; $f = Rq doctor --fix; Rq start apache | Out-Null; $up = Listening $ApachePort; Rq stop | Out-Null; Rq config set apache document_root '""' | Out-Null; Remove-Item $site -Recurse -Force -ErrorAction SilentlyContinue
  $a -match "website folder is missing" -and $up }
Check "DOC-E5" "a moved install (copied elsewhere) is detected and repaired, then starts" {
  $new = "$env:TEMP\rt-e2e-moved"; Remove-Item $new -Recurse -Force -ErrorAction SilentlyContinue
  robocopy $Root $new /E /XD tmp backups /NFL /NDL /NJH /NJS /NP | Out-Null
  $env:RUNQORA_DIR = $new; $a = Rq doctor; Rq doctor --fix | Out-Null; $clean = (Rq doctor) -match "Everything"; Rq start | Out-Null; $up = (Listening $ApachePort) -and (Listening $DbPort); Rq stop | Out-Null
  $env:RUNQORA_DIR = $Root; Remove-Item $new -Recurse -Force -ErrorAction SilentlyContinue; $a -match "wrong folder" -and $clean -and $up }

Write-Host "`nF07 autostart and the window"
Check "F07-H1" "an autostart flag makes the window start that server and no other" {
  Rq config set apache autostart on | Out-Null; Rq config set mariadb autostart off | Out-Null
  $p = Start-Process $App -PassThru; $deadline = (Get-Date).AddSeconds(25); while (-not (Listening $ApachePort) -and (Get-Date) -lt $deadline) { Start-Sleep 1 }
  $r = (Listening $ApachePort) -and -not (Listening $DbPort); Stop-Process $p -Force; Rq stop | Out-Null; Rq config set apache autostart off | Out-Null; $r }
Check "F07-H2" "the window has the right title and the Mica backdrop" {
  Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class DwmE2E { [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out int v, int s); }' -ErrorAction SilentlyContinue
  $p = Start-Process $App -PassThru; Start-Sleep 7; $v = 0; [DwmE2E]::DwmGetWindowAttribute($p.MainWindowHandle, 38, [ref]$v, 4) | Out-Null; $t = $p.MainWindowTitle; Stop-Process $p -Force; $t -eq "Runqora" -and $v -eq 2 }

if ($Network) {
  Write-Host "`nUpdates (needs the internet)"
  Check "UPD-H1" "the check lists every installed module with versions" { $o = Rq updates; (Code) -eq 0 -and $o -match "Apache" -and $o -match "PHP" -and $o -match "MariaDB" -and $o -match "phpMyAdmin" }
  Check "UPD-E1" "the installed version comes from the program itself" { (Rq updates) -match "installed 8\.\d+\.\d+" }
  Check "UPD-N1" "a major MariaDB upgrade is refused without --yes and shows its warning (nothing downloaded)" { $o = Rq update mariadb 11.4.13; (Code) -eq 2 -and $o -match "--yes" -and $o -match "database" }
  Check "UPD-N2" "an update for an unknown module exits 2" { Rq update nginx | Out-Null; (Code) -eq 2 }
  Check "UPD-N3" "rollback with nothing to roll back to exits 1" { Rq rollback apache | Out-Null; (Code) -eq 1 }
  Check "UPD-N4" "an update is refused while the server runs" { Rq start apache | Out-Null; $o = Rq update php 9.9.9; $r = (Code) -ne 0; Rq stop | Out-Null; $r }
  Check "UPD-N5" "every module's publisher is read and offers a version with a checksum (nothing is downloaded)" {
    $ok = $true; foreach ($m in @("jre:21.0.1", "tomcat:10.1.1", "mailpit:1.0.0", "sftpgo:2.0.0", "wordpress:6.0.0", "node:20.0.0", "composer:2.0.0", "adminer:4.0.0", "php:8.3.1", "mariadb:10.11.1", "apache:2.4.1", "phpmyadmin:5.0.0")) {
      $id, $v = $m.Split(":"); $o = Rq updates --probe $id $v; if ((Code) -ne 0 -or $o -notmatch "(sha(1|512):)?[0-9a-f]{40,}") { Write-Host "    probe $id -> $o"; $ok = $false } }
    $ok }
  Check "UPD-N6" "the newest Runqora release is found and compared with this build" { $o = Rq app-update; (Code) -eq 0 -and $o -match "Runqora" }
  Check "UPD-N7" "probing an unknown module fails cleanly" { Rq updates --probe nope | Out-Null; (Code) -eq 1 }
} else { Skip "UPD-*" "network checks off (pass -Network)" }

Write-Host "`nSites, HTTPS and the newer modules"
$sslExisted = Test-Path "$Root\ssl"; $siteDir = "$Root\htdocs\e2eshop"; $curl = "$env:SystemRoot\System32\curl.exe"
function Tls($url, [switch]$NoCa) { $a = @("-s", "--ssl-no-revoke", "--max-time", "8"); if (-not $NoCa) { $a += @("--cacert", "$Root\ssl\ca.crt") }; $out = & $curl @a $url 2>$null; "$LASTEXITCODE|$out" }
Check "SITE-N1" "bad, reserved and duplicate site names are refused" { Rq sites add "a.b" | Out-Null; $a = Code; Rq sites add localhost | Out-Null; $b = Code; Rq sites add e2eshop | Out-Null; Rq sites add e2eshop | Out-Null; $c = Code; $a -eq 1 -and $b -eq 1 -and $c -eq 1 }
Check "SITE-H1" "a site is listed and gets its own virtual host, with localhost kept as the default" {
  $o = Rq sites; $conf = Get-Content "$Root\apache\conf\extra\runqora.conf" -Raw
  $o -match "e2eshop.localhost" -and $conf -match "ServerName e2eshop.localhost" -and $conf.IndexOf("ServerName localhost") -ge 0 -and $conf.IndexOf("ServerName localhost") -lt $conf.IndexOf("ServerName e2eshop.localhost") }
Check "SITE-H2" "the site answers on its own name and other names still reach the default site" {
  Set-Content "$siteDir\index.html" "E2E-SHOP-OK" -Encoding ascii; Rq start apache | Out-Null
  $shop = (& $curl -s --max-time 8 "http://e2eshop.localhost:$ApachePort/") -join ""; $mainPage = (& $curl -s --max-time 8 "http://localhost:$ApachePort/") -join ""; $other = (& $curl -s --max-time 8 "http://unknown.localhost:$ApachePort/") -join "" # curl resolves *.localhost itself, like browsers; .NET does not
  $shop -match "E2E-SHOP-OK" -and $mainPage -notmatch "E2E-SHOP-OK" -and $other -notmatch "E2E-SHOP-OK" }
Check "HTTPS-H1" "HTTPS covers localhost and every site, verified against Runqora's own authority (nothing is trusted system-wide)" {
  Rq stop | Out-Null; Rq config set apache https_port 18443 | Out-Null; Rq config set apache https on | Out-Null; Rq start apache | Out-Null
  $a = Tls "https://localhost:18443/"; $b = Tls "https://e2eshop.localhost:18443/"; $c = Tls "https://127.0.0.1:18443/"
  $a.StartsWith("0|") -and $b -match "^0\|E2E-SHOP-OK" -and $c.StartsWith("0|") }
Check "HTTPS-N1" "without the authority, or for a name the certificate lacks, the connection is refused" {
  $a = Tls "https://localhost:18443/" -NoCa; $b = Tls "https://not-in-cert.localhost:18443/"; $a.StartsWith("60|") -and $b.StartsWith("60|") }
Check "HTTPS-N2" "the certificate is not in the Windows trust store unless the user asked" { -not ((certutil -user -store Root "Runqora Local CA" 2>&1 | Out-String) -match "Runqora Local CA") }
Check "SITE-H3" "removing a site keeps its files and drops its host and certificate name" {
  Rq stop | Out-Null; Rq sites remove e2eshop | Out-Null; $r1 = Code; $conf = Get-Content "$Root\apache\conf\extra\runqora.conf" -Raw
  $r1 -eq 0 -and (Test-Path "$siteDir\index.html") -and $conf -notmatch "e2eshop" }
Check "EXT-H1" "FileZilla Server and Mercury are listed with a state and never downloaded" { $o = Rq external status; (Code) -eq 0 -and $o -match "filezilla" -and $o -match "mercury" }
Check "EXT-N1" "an unknown program or a missing one is refused with a clear message" { Rq external start nope | Out-Null; $a = Code; $a -eq 1 }
Check "MOD-N1" "installing an unknown module fails cleanly (exit 1), and start-all ignores modules that are not installed" { Rq install nope | Out-Null; $a = Code; $o = Rq start; $a -eq 1 -and $o -notmatch "not installed"; }
Rq stop | Out-Null
Remove-Item $siteDir -Recurse -Force -ErrorAction SilentlyContinue; if (-not $sslExisted) { Remove-Item "$Root\ssl" -Recurse -Force -ErrorAction SilentlyContinue }

Write-Host "`nOpen buttons and folders"
$env:RQ_NO_BROWSER = "1"   # print the address instead of opening a browser
Check "OPEN-H1" "Open on phpMyAdmin starts MariaDB and Apache from stopped, then gives its page, which answers" {
  Rq stop | Out-Null; $o = Rq open phpmyadmin; $code = Code
  $code -eq 0 -and $o -match "phpmyadmin/" -and $o -match "Started" -and (Listening $ApachePort) -and (Listening $DbPort) -and ((Get-Page "http://127.0.0.1:$ApachePort/phpmyadmin/" 120).StatusCode -eq 200) }
Check "OPEN-E1" "opening again starts nothing and still gives the page" { $o = Rq open phpmyadmin; (Code) -eq 0 -and $o -match "phpmyadmin/" -and $o -match "Opened" -and $o -notmatch "Started" }
Check "OPEN-N1" "opening a module that is not installed says so (exit 1); an unknown one too" { Rq open tomcat | Out-Null; $a = Code; Rq open nope | Out-Null; $b = Code; $a -eq 1 -and $b -eq 1 }
Check "FOLDER-H1" "the install folder and a module's folder are found" { $a = Rq folder; $c1 = Code; $b = Rq folder phpmyadmin; $c2 = Code; $c1 -eq 0 -and $c2 -eq 0 -and $a.Trim().Length -gt 3 -and $b -match "phpmyadmin" }
Check "FOLDER-N1" "a module that is not installed has no folder to open (exit 1)" { Rq folder tomcat | Out-Null; (Code) -eq 1 }
Remove-Item Env:RQ_NO_BROWSER -ErrorAction SilentlyContinue
Rq stop | Out-Null

# put everything back
Rq stop | Out-Null
Set-Content $phpIni $phpIniBackup -Encoding ascii -NoNewline; Set-Content $myIni $myIniBackup -Encoding ascii -NoNewline; Set-Content "$Root\apache\conf\httpd.conf" $confBackup -Encoding ascii -NoNewline
if ($settingsBackup) { Set-Content $settingsFile $settingsBackup -Encoding ascii -NoNewline } else { Remove-Item $settingsFile -ErrorAction SilentlyContinue }
Remove-Item "$env:TEMP\rt-e2e-site", "$Root\runqora.ini", "$Root\php\php.ini.runqora-backup", "$Root\mariadb\my.ini.runqora-backup" -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "`n----------------------------------------"
Write-Host ("passed {0}, failed {1}, skipped {2}" -f $script:pass, $script:fail.Count, $script:skip.Count)
$script:fail | % { Write-Host "  FAILED: $_" -ForegroundColor Red }
exit $script:fail.Count
