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
Copy-Item (Join-Path $TmpDir "axon-runtime.exe") (Join-Path $InstallDir "axon-runtime.exe") -Force

# Install dendrite bridge
if (-not (Test-Path $NativeDir)) {
    New-Item -ItemType Directory -Path $NativeDir -Force | Out-Null
}
$BridgeLib = Get-ChildItem $TmpDir -Filter "libaxon_dendrite_bridge.*" | Select-Object -First 1
if ($BridgeLib) {
    Copy-Item $BridgeLib.FullName (Join-Path $NativeDir $BridgeLib.Name) -Force
    # Set environment variable
    [Environment]::SetEnvironmentVariable(
        "EASYNET_DENDRITE_BRIDGE_LIB",
        (Join-Path $NativeDir $BridgeLib.Name),
        "User"
    )
    Write-Host "Set EASYNET_DENDRITE_BRIDGE_LIB environment variable"
}

# Add to PATH if not already there
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    Write-Host "Added $InstallDir to PATH (restart your terminal to take effect)"
}

# Cleanup
Remove-Item $TmpZip -Force
Remove-Item $TmpDir -Recurse -Force

Write-Host ""
Write-Host "  EasyNet CLI installed to $InstallDir"
Write-Host "    - easynet.exe"
Write-Host "    - axon-runtime.exe"
Write-Host "    - dendrite bridge -> $NativeDir"
Write-Host ""
Write-Host "  Run 'easynet --help' to get started."
Write-Host ""
