$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Target = Join-Path $Root "engineering/scripts/build-windows-cli.ps1"
& $Target @args
exit $LASTEXITCODE
