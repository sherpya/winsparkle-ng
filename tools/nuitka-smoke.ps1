param([string]$Python='python')
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$stage=Join-Path $root 'artifacts\nuitka-input'
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $root 'bindings\python\winsparkle.py') $stage
Copy-Item (Join-Path $root 'examples\python-nuitka\abi_probe.py') $stage
Copy-Item (Join-Path $root 'target\x86_64-pc-windows-msvc\release\WinSparkle.dll') $stage
$env:PYTHONPATH=$stage
& $Python -m nuitka --standalone --msvc=latest --assume-yes-for-downloads --include-module=winsparkle "--user-package-configuration-file=$root\examples\python-nuitka\package.yml" "--output-dir=$root\artifacts\nuitka" (Join-Path $stage 'abi_probe.py')
if ($LASTEXITCODE -ne 0) {throw 'Nuitka compilation failed'}
& (Join-Path $root 'artifacts\nuitka\abi_probe.dist\abi_probe.exe')
if ($LASTEXITCODE -ne 0) {throw 'Nuitka standalone probe failed'}
