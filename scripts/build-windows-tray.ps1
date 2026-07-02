$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Target = Join-Path $Root "engineering/scripts/build-windows-tray.ps1"
& $Target @args
exit $LASTEXITCODE
