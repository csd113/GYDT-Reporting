[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'Run this script on Windows with PowerShell 7.' }

$archive = [IO.Path]::GetFullPath($OutputPath)
if ([IO.Path]::GetExtension($archive) -ne '.zip' -or (Test-Path -LiteralPath $archive)) {
    throw 'Choose a new output path ending in .zip.'
}
$directory = [IO.Path]::GetDirectoryName($archive)
if (-not [IO.Directory]::Exists($directory)) { throw 'The output directory must exist.' }

$root = Split-Path -Parent $PSScriptRoot
$expected = @('data/substances-catalog.xml', 'templates/results-email.eml', 'templates/results-sms.txt')
$seen = @{}
foreach ($row in Get-Content -LiteralPath (Join-Path $root 'scripts/synthetic-inputs.sha256')) {
    $line = [regex]::Match($row, '^([0-9a-f]{64})  ([^ ]+)$')
    $name = $line.Groups[2].Value
    if (-not $line.Success -or $name -cnotin $expected -or $seen.ContainsKey($name)) {
        throw 'The synthetic input list is invalid.'
    }
    $hash = $line.Groups[1].Value
    $actual = (Get-FileHash -LiteralPath (Join-Path $root $name) -Algorithm SHA256).Hash
    if ($hash -ine $actual) { throw 'Only the reviewed synthetic catalogue and draft templates can be packaged here.' }
    $seen[$name] = $true
}
if ($seen.Count -ne $expected.Count) { throw 'The synthetic input list is incomplete.' }

$triple = 'x86_64-pc-windows-msvc'
$target = Join-Path $root 'target/gydt-windows-profile'
$profile = [IO.Path]::GetFullPath([Environment]::GetFolderPath('UserProfile'))
$cargoHome = if ($env:CARGO_HOME) {
    [IO.Path]::GetFullPath($env:CARGO_HOME, $root)
} else {
    Join-Path $profile '.cargo'
}
$privateRoots = @($root, $profile, $cargoHome)
$remaps = @()
foreach ($entry in @(
    @{ Path = $profile; Public = '/build-home/' },
    @{ Path = $cargoHome; Public = '/cargo/' },
    @{ Path = $root; Public = '/gydt-source/' }
) | Sort-Object -Property { $_.Path.Length }) {
    $native = $entry.Path.TrimEnd('\', '/')
    $remaps += "--remap-path-prefix=$native\=$($entry.Public)"
    $remaps += "--remap-path-prefix=$($native.Replace('\', '/'))/=$($entry.Public)"
}
$remaps += '--remap-path-scope=object'

# Cargo's encoded form preserves spaces in a Windows home or workspace path.
# Apply the remaps to dependencies as well as the app.
$originalEncoded = $env:CARGO_ENCODED_RUSTFLAGS
$existing = if ($null -ne $originalEncoded) {
    $originalEncoded.Split([char]31)
} elseif ($null -ne $env:RUSTFLAGS) {
    $env:RUSTFLAGS -split '\s+'
} elseif ($env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS) {
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS -split '\s+'
}
$env:CARGO_ENCODED_RUSTFLAGS = (@($existing | Where-Object { $_ }) + $remaps) -join [char]31
Push-Location -LiteralPath $root
try {
    cargo build --release --locked --no-default-features --target $triple --bin gydt-results-compiler --target-dir $target
    if ($LASTEXITCODE -ne 0) { throw 'The Windows build failed.' }
} finally {
    Pop-Location
    $env:CARGO_ENCODED_RUSTFLAGS = $originalEncoded
}
$compiled = Join-Path $target "$triple/release/gydt-results-compiler.exe"
$binaryBytes = [IO.File]::ReadAllBytes($compiled)
$asciiView = [Text.Encoding]::Latin1.GetString($binaryBytes)
$wideView = [Text.Encoding]::Unicode.GetString($binaryBytes)
$wideOddView = [Text.Encoding]::Unicode.GetString($binaryBytes, 1, $binaryBytes.Length - 1)
foreach ($private in $privateRoots) {
    foreach ($pathForm in @($private, $private.Replace('\', '/'))) {
        foreach ($view in @($asciiView, $wideView, $wideOddView)) {
            if ($view.Contains($pathForm, [StringComparison]::OrdinalIgnoreCase)) {
                throw 'The Windows executable still contains a private builder or workspace path.'
            }
        }
    }
}
# Check the PE machine type before labelling the package x64.
$reader = [IO.BinaryReader]::new([IO.File]::OpenRead($compiled))
try {
    if ($reader.ReadUInt16() -ne 0x5a4d) { throw 'The application is not a Windows executable.' }
    $reader.BaseStream.Position = 0x3c
    $reader.BaseStream.Position = $reader.ReadUInt32()
    if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) {
        throw 'Expected a Windows x64 executable.'
    }
} finally {
    $reader.Dispose()
}

$stage = Join-Path $directory ('.gydt-windows-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($stage) | Out-Null
try {
    $name = 'GYDT Sample Reporting.exe'
    $binary = Join-Path $stage $name
    $zip = Join-Path $stage 'application.zip'
    Copy-Item -LiteralPath $compiled -Destination $binary
    Compress-Archive -LiteralPath $binary -DestinationPath $zip -CompressionLevel Optimal
    $ready = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        if ($ready.Entries.Count -ne 1 -or
            $ready.Entries[0].FullName -cne $name -or
            $ready.Entries[0].Length -ne (Get-Item -LiteralPath $binary).Length) {
            throw 'The package must contain only the GYDT application executable.'
        }
    } finally {
        $ready.Dispose()
    }
    [IO.File]::Move($zip, $archive)
    Write-Output $archive
} finally {
    Remove-Item -LiteralPath $stage -Force -Recurse
}
