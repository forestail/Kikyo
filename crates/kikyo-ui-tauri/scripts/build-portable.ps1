$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectDir = Resolve-Path (Join-Path $scriptDir "..")
$workspaceDir = Resolve-Path (Join-Path $projectDir "..\..")

Push-Location $projectDir
try {
    npm run tauri -- build --no-bundle --features portable-mode
}
finally {
    Pop-Location
}

$tauriConfigPath = Join-Path $projectDir "src-tauri\tauri.conf.json"
$tauriConfig = Get-Content -Path $tauriConfigPath -Raw | ConvertFrom-Json
$productName = [string]$tauriConfig.productName
$version = [string]$tauriConfig.version

$binaryPath = Join-Path $workspaceDir "target\release\kikyo.exe"
if (-not (Test-Path -Path $binaryPath)) {
    throw "Portable binary not found: $binaryPath"
}

$portableRoot = Join-Path $workspaceDir "target\release\bundle\portable"
$portableName = "{0}_{1}_x64-portable" -f $productName, $version
$portableDir = Join-Path $portableRoot $portableName
$zipPath = Join-Path $portableRoot ("{0}.zip" -f $portableName)

Remove-Item -Path $portableDir -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -Path $zipPath -Force -ErrorAction SilentlyContinue
New-Item -Path $portableDir -ItemType Directory -Force | Out-Null

Copy-Item -Path $binaryPath -Destination (Join-Path $portableDir "kikyo.exe") -Force

$portableReadmePath = Join-Path $portableDir "README-portable.txt"
@"
Kikyo Portable
==============

- Extract this zip to a normal folder first, then run kikyo.exe.
- settings.json is read/written only in this same directory.
- This portable build never reads/writes AppData settings.
- Microsoft Edge WebView2 Runtime is required.
"@ | Set-Content -Path $portableReadmePath -Encoding utf8

Compress-Archive -Path (Join-Path $portableDir "*") -DestinationPath $zipPath -Force
Write-Host "Portable package created: $zipPath"
