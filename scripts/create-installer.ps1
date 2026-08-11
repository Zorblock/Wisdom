$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$compiler = Join-Path $root '.tools\inno\ISCC.exe'
$script = Join-Path $root 'installer\Wisdom.iss'
$package = Get-Content (Join-Path $root 'package.json') -Raw | ConvertFrom-Json
$env:npm_package_version = $package.version

if (-not (Test-Path $compiler)) {
    throw "Inno Setup compiler not found: $compiler"
}

& $compiler $script
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
