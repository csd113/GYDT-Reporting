[CmdletBinding()]
param([Parameter(Mandatory)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
$exe = [IO.Path]::GetFullPath($OutputPath)
if (Test-Path -LiteralPath $exe) { throw 'Choose a new camera helper output path.' }
$dir = [IO.Path]::GetDirectoryName($exe)
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'Visual C++ x64 and the Windows SDK are required to build webcam support.' }
& (Join-Path $vs 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Host
$obj = Join-Path $dir ('camera-' + [guid]::NewGuid().ToString('N') + '.obj')
try {
    & cl /nologo /std:c++20 /EHsc /O2 /MT /W4 /WX "/Fo:$obj" "/Fe:$exe" (Join-Path $PSScriptRoot 'GYDT-camera.cpp') /link windowsapp.lib | Out-Host
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $exe)) { throw 'Camera helper compilation failed.' }
} finally {
    if (Test-Path -LiteralPath $obj) { Remove-Item -LiteralPath $obj -Force }
}
