# EasyNet CLI installer for Windows
# Usage: irm https://easynet.run/install.ps1 | iex
$ErrorActionPreference = "Stop"

$BaseUrl = "https://easynet.run/download"
$InstallDir = "$env:USERPROFILE\.easynet\bin"
$NativeDir = "$env:USERPROFILE\.easynet\dendrite-bridge\native"
$Target = "x86_64-pc-windows-gnu"

# Download
$Url = "$BaseUrl/easynet-$Target.zip"
$TmpZip = Join-Path $env:TEMP "easynet.zip"
$TmpDir = Join-Path $env:TEMP "easynet-install"

Write-Host "Downloading $Url..."
Invoke-WebRequest -Uri $Url -OutFile $TmpZip

# Extract
if (Test-Path $TmpDir) { Remove-Item $TmpDir -Recurse -Force }
Expand-Archive -Path $TmpZip -DestinationPath $TmpDir

# Install binaries
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}
Copy-Item (Join-Path $TmpDir "easynet.exe") (Join-Path $InstallDir "easynet.exe") -Force
Copy-Item (Join-Path $TmpDir "easynet-daemon.exe") (Join-Path $InstallDir "easynet-daemon.exe") -Force
Copy-Item (Join-Path $TmpDir "easynet-keyring.exe") (Join-Path $InstallDir "easynet-keyring.exe") -Force
Copy-Item (Join-Path $TmpDir "easynet-remoteapp-native-host.exe") (Join-Path $InstallDir "easynet-remoteapp-native-host.exe") -Force
Copy-Item (Join-Path $TmpDir "easynet-remoteapp-media-host.exe") (Join-Path $InstallDir "easynet-remoteapp-media-host.exe") -Force

# Install dendrite bridge
if (-not (Test-Path $NativeDir)) {
    New-Item -ItemType Directory -Path $NativeDir -Force | Out-Null
}
$BridgeLib = Get-ChildItem $TmpDir -Filter "libaxon_dendrite_bridge.*" | Select-Object -First 1
if ($BridgeLib) {
    Copy-Item $BridgeLib.FullName (Join-Path $NativeDir $BridgeLib.Name) -Force
    $BridgePath = Join-Path $NativeDir $BridgeLib.Name
    # Persist for future sessions
    [Environment]::SetEnvironmentVariable("EASYNET_DENDRITE_BRIDGE_LIB", $BridgePath, "User")
    # Activate in current session immediately
    $env:EASYNET_DENDRITE_BRIDGE_LIB = $BridgePath
    Write-Host "Set EASYNET_DENDRITE_BRIDGE_LIB = $BridgePath"
}

# Add to PATH if not already there
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    # Activate in current session immediately
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "Added $InstallDir to PATH"
}

# Remove stale binaries from other PATH dirs that would shadow the install
$PathDirs = $env:Path -split ";"
foreach ($bin in @("easynet.exe", "easynet-daemon.exe", "easynet-keyring.exe", "easynet-remoteapp-native-host.exe", "easynet-remoteapp-media-host.exe", "axon-runtime.exe")) {
    foreach ($dir in $PathDirs) {
        if ($dir -eq $InstallDir) { continue }
        $candidate = Join-Path $dir $bin
        if (Test-Path $candidate) {
            Write-Host "  Removing stale $candidate (shadows $InstallDir\$bin)"
            Remove-Item $candidate -Force -ErrorAction SilentlyContinue
        }
    }
}

# Cleanup
Remove-Item $TmpZip -Force
Remove-Item $TmpDir -Recurse -Force

Write-Host ""
Write-Host "  ✓ EasyNet CLI installed successfully!"
Write-Host ""
Write-Host "    easynet.exe         → $InstallDir"
Write-Host "    easynet-daemon.exe  → $InstallDir"
Write-Host "    easynet-keyring.exe → $InstallDir"
Write-Host "    easynet-remoteapp-native-host.exe → $InstallDir"
Write-Host "    easynet-remoteapp-media-host.exe → $InstallDir"
Write-Host "    dendrite bridge     → $NativeDir"
Write-Host ""
Write-Host "  Run 'easynet --help' to get started."
Write-Host ""
