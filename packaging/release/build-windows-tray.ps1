param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",
    [string]$RuntimeIdentifier = "win-x64"
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Project = Join-Path $Root "platforms\windows\EasyNetTray\EasyNetTray.csproj"
$Dist = Join-Path $Root "plugins\desktop-menubar\dist\windows\EasyNetTray"
$Exe = Join-Path $Dist "EasyNetTray.exe"

if (Test-Path $Dist) {
    Remove-Item $Dist -Recurse -Force
}

dotnet publish $Project `
    -c $Configuration `
    -r $RuntimeIdentifier `
    --self-contained false `
    -p:EnableWindowsTargeting=true `
    -o $Dist

if (!(Test-Path $Exe)) {
    throw "Expected Windows tray artifact was not produced: $Exe"
}

Write-Output $Dist
