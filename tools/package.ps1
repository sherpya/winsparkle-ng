param([ValidateSet('x64','x86','arm64')][string]$Arch='x64',[switch]$SkipBuild)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
Set-Location $root
$target=@{x64='x86_64-pc-windows-msvc';x86='i686-pc-windows-msvc';arm64='aarch64-pc-windows-msvc'}[$Arch]
$metadata=& cargo metadata --locked --no-deps --format-version 1
if ($LASTEXITCODE -ne 0) {throw 'Unable to read package version'}
$version=($metadata | ConvertFrom-Json).packages | Where-Object {$_.name -eq 'winsparkle-ng'} | Select-Object -ExpandProperty version
if (-not $SkipBuild) {
    & cargo build --locked --release --target $target
    if ($LASTEXITCODE -ne 0) {throw 'Release build failed'}
}
$tree=& cargo tree --locked -p winsparkle-ng --target $target -e normal,build
if ($LASTEXITCODE -ne 0 -or ($tree -join "`n") -match '(?i)openssl|libssl|libcrypto') {throw 'Dependency audit failed'}
$destination=Join-Path $root "dist\$Arch"
New-Item -ItemType Directory -Force $destination | Out-Null
Copy-Item "target\$target\release\WinSparkle.dll" "$destination\WinSparkle.dll"
Copy-Item "target\$target\release\WinSparkle.dll.lib" "$destination\WinSparkle.lib"
Copy-Item include\winsparkle.h,include\winsparkle-version.h,LICENSE,bindings\python\winsparkle.py $destination
$packageReadme=(Get-Content (Join-Path $PSScriptRoot 'package-readme.md') -Raw).Replace('{{VERSION}}',$version).Replace('{{ARCH}}',$Arch)
[System.IO.File]::WriteAllText((Join-Path $destination 'README.md'),$packageReadme,[System.Text.UTF8Encoding]::new($false))
New-Item -ItemType Directory -Force artifacts | Out-Null
& python tools\pe_report.py "$destination\WinSparkle.dll" --output "artifacts\pe-$Arch.json"
if ($LASTEXITCODE -ne 0) {throw 'PE audit failed'}
$report=Get-Content "artifacts\pe-$Arch.json" -Raw | ConvertFrom-Json
if (($report.imports -join "`n") -match '(?i)libcurl|libssl|libcrypto|openssl|wx|vcruntime|msvcp') {throw 'Unexpected runtime dependency'}
$header=Get-Content include\winsparkle.h -Raw
$expected=[regex]::Matches($header,'WIN_SPARKLE_API\s+\w+\s+__cdecl\s+(win_sparkle_\w+)\s*\(') | ForEach-Object {$_.Groups[1].Value}
foreach ($name in $expected) {if ($name -notin $report.exports) {throw "Missing export: $name"}}
$tree | Set-Content "artifacts\cargo-tree-$Arch.txt"
$archive=Join-Path $root "dist\WinSparkle-NG-$version-$Arch.zip"
$packageFiles=@('WinSparkle.dll','WinSparkle.lib','winsparkle.h','winsparkle-version.h','winsparkle.py','LICENSE','README.md') | ForEach-Object {Join-Path $destination $_}
Compress-Archive -LiteralPath $packageFiles -DestinationPath $archive -Force
$metrics=[ordered]@{architecture=$Arch;dll_bytes=$report.raw_bytes;zip_bytes=(Get-Item $archive).Length;exports=$report.export_count;stringtable_bytes=$report.stringtable_bytes;resource_languages=$report.stringtable_languages.Count;http='WinHTTP / Schannel';tokio=$false;ed25519=$true;imports=$report.imports}
$metrics | ConvertTo-Json | Set-Content "artifacts\metrics-$Arch.json"
$metrics | ConvertTo-Json
Write-Output "Package: $archive"
