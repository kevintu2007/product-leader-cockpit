#Requires -Version 5.1
<#
.SYNOPSIS
  Proves the PMC installer's uninstaller deletes no data (item 10).

.DESCRIPTION
  Run in a disposable local Windows account made for this test, never in an
  account whose PMC data matters: it installs PMC for the current user, asks
  you to open it once, uninstalls it, and checks that every file in the
  folders below is still there and unchanged: PMC's data folder, both WebView
  data folders under the bundle identifier, and a stand-in for a folder the
  person chose. It does not look anywhere else.

  Turn the network off before running it: the report records whether it was
  off while installing (the installer embeds WebView2; on Windows 11 WebView2
  is already present, so the embedded installer does not run at all).

  1. Checks the installer against its .sha256 file.
  2. Seeds probe files in PMC's data folder, in both WebView data folders
     under the bundle identifier, and in a stand-in for a Vault / backup
     folder the person chose.
  3. Installs silently (/S) and checks the installed executable's build
     metadata.
  4. Opens PMC once: choose "Start with my workspace", let PMC restart, then
     close it. This writes real settings and a real Ledger.
  5. Hashes every file under those folders, uninstalls silently, and hashes
     them again: nothing may be missing or changed.
  6. Checks what the uninstaller must remove: the install folder, the
     uninstall registration, the Start Menu and desktop shortcuts, the Run
     value and the installer's own registry key.

  Writes installer-data-preservation-report.json next to the installer and
  exits 1 on any failure.

.EXAMPLE
  .\installer-data-preservation.ps1 -Installer .\product-mission-control_0.2.0-beta_windows-x86_64_nsis-setup.exe -DisposableAccount
#>
param(
  [Parameter(Mandatory = $true)][string]$Installer,
  # Required: says this is the disposable account the test is meant for.
  [switch]$DisposableAccount
)

$ErrorActionPreference = "Stop"
if (-not $DisposableAccount) {
  throw "Run this only in a disposable Windows account made for the test, and pass -DisposableAccount to say so."
}

$productName = "Product Mission Control"
$bundleId = "com.productmissioncontrol.desktop"
$publisher = "Product Mission Control"
$dataRoot = Join-Path $env:LOCALAPPDATA "ProductMissionControlDesktop"
$webviewLocal = Join-Path $env:LOCALAPPDATA $bundleId
$webviewRoaming = Join-Path $env:APPDATA $bundleId
$external = Join-Path $env:USERPROFILE "PMC installer test - chosen folder"
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$productName"
$installerKey = "HKCU:\Software\$publisher\$productName"
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$startMenu = Join-Path ([Environment]::GetFolderPath("Programs")) "$productName.lnk"
$desktopLink = Join-Path ([Environment]::GetFolderPath("Desktop")) "$productName.lnk"

$failures = New-Object System.Collections.Generic.List[string]
function Fail([string]$message) { $failures.Add($message); Write-Host "FAIL  $message" -ForegroundColor Red }
function Pass([string]$message) { Write-Host "ok    $message" -ForegroundColor Green }

# 1. The checksum ---------------------------------------------------------
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$checksumFile = "$installerPath.sha256"
if (-not (Test-Path -LiteralPath $checksumFile)) { throw "No checksum file beside the installer: $checksumFile" }
$expected = ((Get-Content -LiteralPath $checksumFile -Raw).Trim() -split "\s+")[0].ToLowerInvariant()
$actual = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($expected -ne $actual) { throw "The installer does not match its SHA-256: expected $expected, found $actual" }
Pass "installer SHA-256 $actual"

if (Test-Path $uninstallKey) { throw "PMC is already installed in this account; uninstall it first." }

# 2. Probe files ------------------------------------------------------------
function Write-Probe([string]$folder) {
  $probe = Join-Path $folder "installer-probe"
  New-Item -ItemType Directory -Force -Path $probe | Out-Null
  Set-Content -LiteralPath (Join-Path $probe "probe.txt") -Value "installer probe $(Get-Date -Format o)" -Encoding UTF8
}
foreach ($folder in @($dataRoot, $webviewLocal, $webviewRoaming, $external)) { Write-Probe $folder }
Pass "probe files written"

function Get-Snapshot {
  $snapshot = @{}
  foreach ($folder in @($dataRoot, $webviewLocal, $webviewRoaming, $external)) {
    if (-not (Test-Path -LiteralPath $folder)) { continue }
    Get-ChildItem -LiteralPath $folder -Recurse -File -Force | ForEach-Object {
      try {
        $snapshot[$_.FullName] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
      } catch {
        $snapshot[$_.FullName] = "unreadable"
      }
    }
  }
  return $snapshot
}

# 3. Install ----------------------------------------------------------------
$networkAnswer = Read-Host "Is this computer's network turned off now? (y/n)"
$networkOff = $networkAnswer -match '^(y|yes)$'
$webview2Key = "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
$webview2Present = $null -ne (Get-ItemProperty -Path $webview2Key -ErrorAction SilentlyContinue)
if (-not $networkOff) { Write-Host "note  installing with the network on: this run does not show the installer works offline" -ForegroundColor Yellow }
$install = Start-Process -FilePath $installerPath -ArgumentList "/S" -PassThru -Wait
if ($install.ExitCode -ne 0) { throw "The installer exited with $($install.ExitCode)." }
$registration = Get-ItemProperty -Path $uninstallKey
$installDir = $registration.InstallLocation.Trim('"')
$executable = Join-Path $installDir "pmc-desktop.exe"
if (-not (Test-Path -LiteralPath $executable)) { throw "Installed, but $executable is missing." }
Pass "installed to $installDir"
$metadata = & $executable --pmc-build-metadata | ConvertFrom-Json
if ($metadata.dirty -ne "false") { Fail "the installed executable reports dirty=$($metadata.dirty)" } else { Pass "installed executable built from commit $($metadata.commit), clean" }

# 4. Open PMC once ----------------------------------------------------------
Write-Host ""
Write-Host "PMC opens now. Choose 'Start with my workspace', wait for it to restart," -ForegroundColor Cyan
Write-Host "then close PMC. Press Enter here once PMC is closed." -ForegroundColor Cyan
Start-Process -FilePath $executable | Out-Null
[void](Read-Host)
if (Get-Process -Name "pmc-desktop" -ErrorAction SilentlyContinue) { throw "PMC is still running; close it and run the test again." }
if (-not (Test-Path -LiteralPath (Join-Path $dataRoot "settings-v1.json"))) {
  Fail "PMC's settings were not written; the data check below covers only the probe files"
}

# 5. Uninstall, then compare -----------------------------------------------
$before = Get-Snapshot
Pass "$($before.Count) data files hashed before uninstalling"
$uninstaller = $registration.UninstallString.Trim('"')
Start-Process -FilePath $uninstaller -ArgumentList "/S" -Wait | Out-Null
# The NSIS uninstaller runs from a temporary copy; wait for it to finish.
$deadline = (Get-Date).AddMinutes(2)
while ((Test-Path $uninstallKey) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
Start-Sleep -Seconds 2
$after = Get-Snapshot
$missing = @($before.Keys | Where-Object { -not $after.ContainsKey($_) })
$changed = @($before.Keys | Where-Object { $after.ContainsKey($_) -and $after[$_] -ne $before[$_] })
if ($missing.Count -gt 0) { Fail "uninstall removed $($missing.Count) data file(s): $($missing -join '; ')" } else { Pass "no data file removed" }
if ($changed.Count -gt 0) { Fail "uninstall changed $($changed.Count) data file(s): $($changed -join '; ')" } else { Pass "no data file changed" }

# 6. What the uninstaller must remove -----------------------------------------
$leftovers = @(
  @{ Name = "uninstall registration"; Present = (Test-Path $uninstallKey) },
  @{ Name = "installer registry key"; Present = (Test-Path $installerKey) },
  @{ Name = "install folder $installDir"; Present = (Test-Path -LiteralPath $installDir) },
  @{ Name = "Start Menu shortcut"; Present = (Test-Path -LiteralPath $startMenu) },
  @{ Name = "desktop shortcut"; Present = (Test-Path -LiteralPath $desktopLink) },
  @{ Name = "Run value"; Present = ($null -ne (Get-ItemProperty -Path $runKey -Name $productName -ErrorAction SilentlyContinue)) }
)
foreach ($item in $leftovers) {
  if ($item.Present) { Fail "left behind: $($item.Name)" } else { Pass "removed: $($item.Name)" }
}

$report = [ordered]@{
  installer = (Split-Path -Leaf $installerPath)
  installerSha256 = $actual
  build = $metadata
  installDir = $installDir
  networkOffDuringInstall = $networkOff
  webview2AlreadyPresent = $webview2Present
  dataFilesChecked = $before.Count
  missing = $missing
  changed = $changed
  failures = @($failures)
  passed = ($failures.Count -eq 0)
  finishedAt = (Get-Date -Format o)
}
$reportPath = Join-Path (Split-Path -Parent $installerPath) "installer-data-preservation-report.json"
$report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $reportPath -Encoding UTF8
Write-Host ""
if ($failures.Count -eq 0) {
  Write-Host "PASS  uninstalling deleted nothing in the checked folders. Report: $reportPath" -ForegroundColor Green
  exit 0
}
Write-Host "FAIL  $($failures.Count) check(s) failed. Report: $reportPath" -ForegroundColor Red
exit 1
