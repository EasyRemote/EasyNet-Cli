<#
.SYNOPSIS
Build EasyNet-Cli on Windows and optionally stage a zip package.

.EXAMPLE
.\scripts\build-windows-cli.ps1 -Configuration Release

Builds the native Windows Rust target and stages:
target\windows-package\<host-target>\.

.EXAMPLE
.\scripts\build-windows-cli.ps1 -Configuration Release -Target x86_64-pc-windows-gnu -RequireBridge

Builds the target name used by install.ps1 release downloads and
requires the sibling EasyNet-Axon dendrite bridge DLL to be staged.
#>

param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",

    [string]$Target = "",

    [string]$Features = "",

    [switch]$NoDefaultFeatures,

    [switch]$SkipBridge,

    [switch]$RequireBridge,

    [switch]$NoZip,

    [string]$BridgeCrate = ""
)

$ErrorActionPreference = "Stop"

function Resolve-RepoRoot {
    return (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
}

function Require-Command {
    param([string]$Name)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found on PATH."
    }
}

function Get-RustHostTarget {
    $hostLine = (& rustc -vV | Where-Object { $_ -like "host:*" } | Select-Object -First 1)
    if (-not $hostLine) {
        throw "Could not determine Rust host target from 'rustc -vV'."
    }
    return ($hostLine -replace "^host:\s*", "").Trim()
}

function Invoke-CargoBuild {
    param(
        [string]$WorkingDirectory,
        [string[]]$CargoArgs
    )
    Push-Location $WorkingDirectory
    try {
        Write-Host "cargo $($CargoArgs -join ' ')"
        & cargo @CargoArgs
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed in $WorkingDirectory"
        }
    }
    finally {
        Pop-Location
    }
}

function Cargo-ArtifactDir {
    param(
        [string]$Root,
        [string]$ExplicitTarget,
        [string]$Profile
    )
    $profileDir = $Profile.ToLowerInvariant()
    if ([string]::IsNullOrWhiteSpace($ExplicitTarget)) {
        return Join-Path (Join-Path $Root "target") $profileDir
    }
    return Join-Path (Join-Path (Join-Path $Root "target") $ExplicitTarget) $profileDir
}

$Root = Resolve-RepoRoot
$WorkspaceRoot = Split-Path -Parent $Root
$Profile = $Configuration.ToLowerInvariant()

Require-Command "cargo"
Require-Command "rustc"
Require-Command "cmake"

$HostTarget = Get-RustHostTarget
$TargetLabel = if ([string]::IsNullOrWhiteSpace($Target)) { $HostTarget } else { $Target }

if ([string]::IsNullOrWhiteSpace($BridgeCrate)) {
    $BridgeCrate = Join-Path $WorkspaceRoot "EasyNet-Axon\core\runtime-rs\dendrite-bridge"
}

$CargoArgs = @(
    "build",
    "--locked",
    "-p", "easynet",
    "--lib",
    "--bin", "easynet",
    "--bin", "easynet-daemon",
    "--bin", "easynet-keyring"
)

$NativeHostCargoArgs = @(
    "build",
    "--locked",
    "-p", "easynet-remoteapp-native-host",
    "--bin", "easynet-remoteapp-native-host"
)

$MediaProbeHostCargoArgs = @(
    "build",
    "--locked",
    "-p", "easynet-remoteapp-media-host",
    "--bin", "easynet-remoteapp-media-host"
)

if ($Configuration -eq "Release") {
    $CargoArgs += "--release"
    $NativeHostCargoArgs += "--release"
    $MediaProbeHostCargoArgs += "--release"
}
if (-not [string]::IsNullOrWhiteSpace($Target)) {
    $CargoArgs += @("--target", $Target)
    $NativeHostCargoArgs += @("--target", $Target)
    $MediaProbeHostCargoArgs += @("--target", $Target)
}
if ($NoDefaultFeatures) {
    $CargoArgs += "--no-default-features"
}
if (-not [string]::IsNullOrWhiteSpace($Features)) {
    $CargoArgs += @("--features", $Features)
}

Write-Host "==> [1/3] Building EasyNet CLI binaries"
Write-Host "    root:   $Root"
Write-Host "    target: $TargetLabel"
Write-Host "    config: $Configuration"
Invoke-CargoBuild -WorkingDirectory $Root -CargoArgs $CargoArgs
Invoke-CargoBuild -WorkingDirectory $Root -CargoArgs $NativeHostCargoArgs
Invoke-CargoBuild -WorkingDirectory $Root -CargoArgs $MediaProbeHostCargoArgs

$CliArtifactDir = Cargo-ArtifactDir -Root $Root -ExplicitTarget $Target -Profile $Profile
$RequiredBins = @("easynet.exe", "easynet-daemon.exe", "easynet-keyring.exe", "easynet-remoteapp-native-host.exe", "easynet-remoteapp-media-host.exe")
foreach ($bin in $RequiredBins) {
    $path = Join-Path $CliArtifactDir $bin
    if (-not (Test-Path $path)) {
        throw "Expected build artifact missing: $path"
    }
}
$CAbiDll = Join-Path $CliArtifactDir "easynet_cli.dll"
if (-not (Test-Path $CAbiDll)) {
    throw "Expected C ABI build artifact missing: $CAbiDll"
}

$StageRoot = Join-Path (Join-Path $Root "target") "windows-package"
$StageDir = Join-Path $StageRoot $TargetLabel
if (Test-Path $StageDir) {
    Remove-Item $StageDir -Recurse -Force
}
New-Item -ItemType Directory -Path $StageDir -Force | Out-Null

foreach ($bin in $RequiredBins) {
    Copy-Item (Join-Path $CliArtifactDir $bin) (Join-Path $StageDir $bin) -Force
}
Copy-Item $CAbiDll (Join-Path $StageDir "easynet_cli.dll") -Force

$BridgeStaged = $false
if (-not $SkipBridge) {
    if (Test-Path (Join-Path $BridgeCrate "Cargo.toml")) {
        Write-Host "==> [2/3] Building Axon dendrite bridge"
        $BridgeArgs = @("build", "--locked", "--lib")
        if ($Configuration -eq "Release") {
            $BridgeArgs += "--release"
        }
        if (-not [string]::IsNullOrWhiteSpace($Target)) {
            $BridgeArgs += @("--target", $Target)
        }
        Invoke-CargoBuild -WorkingDirectory $BridgeCrate -CargoArgs $BridgeArgs

        $BridgeArtifactDir = Cargo-ArtifactDir -Root $BridgeCrate -ExplicitTarget $Target -Profile $Profile
        $bridgeDll = Get-ChildItem -Path $BridgeArtifactDir -Filter "*dendrite_bridge*.dll" -File |
            Sort-Object Length -Descending |
            Select-Object -First 1
        if ($bridgeDll) {
            Copy-Item $bridgeDll.FullName (Join-Path $StageDir $bridgeDll.Name) -Force
            $installerName = Join-Path $StageDir "libaxon_dendrite_bridge.dll"
            if ($bridgeDll.Name -ne "libaxon_dendrite_bridge.dll") {
                Copy-Item $bridgeDll.FullName $installerName -Force
            }
            $BridgeStaged = $true
        }
        elseif ($RequireBridge) {
            throw "Bridge build completed but no *dendrite_bridge*.dll was found under $BridgeArtifactDir."
        }
        else {
            Write-Warning "Bridge build completed but no *dendrite_bridge*.dll was found under $BridgeArtifactDir."
        }
    }
    elseif ($RequireBridge) {
        throw "Bridge crate not found: $BridgeCrate"
    }
    else {
        Write-Warning "Bridge crate not found; skipping bridge build: $BridgeCrate"
    }
}
else {
    Write-Host "==> [2/3] Skipping Axon dendrite bridge"
}

$ContractFiles = @(
    @{ Source = "include\easynet_cli.h"; Destination = "include\easynet_cli.h" },
    @{ Source = "include\easynet_cli.exports.v7"; Destination = "include\easynet_cli.exports.v7" },
    @{ Source = "include\easynet_cli.exports.v8"; Destination = "include\easynet_cli.exports.v8" },
    @{ Source = "include\easynet_cli.exports.v9"; Destination = "include\easynet_cli.exports.v9" },
    @{ Source = "docs\spec\ffi-abi-v7.md"; Destination = "docs\spec\ffi-abi-v7.md" },
    @{ Source = "docs\spec\ffi-abi-v8.md"; Destination = "docs\spec\ffi-abi-v8.md" },
    @{ Source = "docs\spec\ffi-abi-v9.md"; Destination = "docs\spec\ffi-abi-v9.md" }
)
foreach ($Contract in $ContractFiles) {
    $Source = Join-Path $Root $Contract.Source
    if (-not (Test-Path $Source)) {
        throw "Required C ABI contract file missing: $Source"
    }
    $Destination = Join-Path $StageDir $Contract.Destination
    New-Item -ItemType Directory -Path (Split-Path -Parent $Destination) -Force | Out-Null
    Copy-Item $Source $Destination -Force
}

Write-Host "==> [3/3] Staged package"
Write-Host "    path: $StageDir"
Write-Host "    bridge: $(if ($BridgeStaged) { 'staged' } else { 'not staged' })"

if (-not $NoZip) {
    $ZipPath = Join-Path $StageRoot "easynet-$TargetLabel.zip"
    if (Test-Path $ZipPath) {
        Remove-Item $ZipPath -Force
    }
    Compress-Archive -Path (Join-Path $StageDir "*") -DestinationPath $ZipPath -Force
    Write-Host "    zip:  $ZipPath"
}

Write-Host ""
Write-Host "[OK] Windows EasyNet CLI build complete."
Write-Host "     Run: $(Join-Path $StageDir 'easynet.exe') --help"
