param([ValidateSet('x64','x86','arm64')][string]$Arch='x64',[string]$Configuration='release',[string]$HeaderDirectory)
$ErrorActionPreference='Stop'
$root = Split-Path $PSScriptRoot -Parent
$target = @{x64='x86_64-pc-windows-msvc';x86='i686-pc-windows-msvc';arm64='aarch64-pc-windows-msvc'}[$Arch]
$vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -property installationPath
Import-Module (Join-Path $vs 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments "-arch=$Arch -host_arch=x64" | Out-Null
$output = Join-Path $root "artifacts\abi-$Arch"
New-Item -ItemType Directory -Force $output | Out-Null
$dll = Join-Path $root "target\$target\$Configuration\WinSparkle.dll"
Copy-Item -LiteralPath $dll -Destination $output
Copy-Item (Join-Path $root "target\$target\$Configuration\WinSparkle.dll.lib") (Join-Path $output 'WinSparkle.lib')
# Use the distributed header by default; an external header can be tested explicitly.
$header = if ($HeaderDirectory) { (Resolve-Path -LiteralPath $HeaderDirectory).Path } else { Join-Path $root 'include' }
& cl.exe /nologo /W4 /WX /MT "/I$header" (Join-Path $root 'tests\c\abi_smoke.c') "/Fo$output\abi_smoke.obj" "/Fe$output\abi_smoke.exe" /link "/LIBPATH:$output"
if ($LASTEXITCODE -ne 0) { throw 'C ABI consumer failed to compile/link' }
& dumpbin.exe /EXPORTS $dll | Set-Content (Join-Path $output 'exports.txt')
if ($Arch -ne 'arm64' -or $env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    & (Join-Path $output 'abi_smoke.exe')
    if ($LASTEXITCODE -ne 0) {throw 'C ABI consumer failed'}
}
