param(
    [ValidatePattern('^\d+\.\d+\.\d+\.\d+$')][string]$Version = '1.0.10.0',
    [string]$ExecutablePath,
    [string]$DistributionFiles
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    if (!$ExecutablePath) {
        # Reproduce the existing release toolchain without changing source or Store packaging.
        foreach ($arguments in @(
            @('+1.90.0', 'fmt', '--', '--check'),
            @('+1.90.0', 'clippy', '--locked', '--all-targets', '--', '-D', 'warnings'),
            @('+1.90.0', 'test', '--locked'),
            @('+1.90.0', 'build', '--release', '--locked')
        )) {
            & cargo @arguments
            if ($LASTEXITCODE -ne 0) { throw "cargo $arguments failed" }
        }
        $ExecutablePath = Join-Path $root 'target/x86_64-pc-windows-msvc/release/gesturelite.exe'
    }
    $ExecutablePath = (Resolve-Path -LiteralPath $ExecutablePath).Path
    if (!$DistributionFiles) { $DistributionFiles = $root }
    $bytes = [IO.File]::ReadAllBytes($ExecutablePath)
    if ($bytes.Length -lt 256 -or [BitConverter]::ToUInt16($bytes, 0) -ne 0x5a4d) { throw 'Invalid executable' }
    $pe = [BitConverter]::ToInt32($bytes, 0x3c)
    if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550 -or [BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) { throw 'Expected Windows x64 PE executable' }
    $out = Join-Path $root 'dist/github'
    New-Item -ItemType Directory -Force $out | Out-Null
    $stage = Join-Path $out ('portable-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory $stage | Out-Null
    Copy-Item -LiteralPath $ExecutablePath -Destination (Join-Path $stage 'GestureLite.exe')
    foreach ($file in @('README.md', 'THIRD_PARTY_NOTICES.txt', 'config.example.json')) {
        Copy-Item -LiteralPath (Join-Path $DistributionFiles $file) -Destination $stage
    }
    $hash = (Get-FileHash -LiteralPath $ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    [ordered]@{
        distribution_version = $Version
        cargo_package_version = '0.1.0'
        architecture = 'x64'
        executable_sha256 = $hash
        authenticode_signed = $false
        configuration = '%LOCALAPPDATA%\GestureLite'
        microsoft_store_package_included = $false
    } | ConvertTo-Json | Set-Content (Join-Path $stage 'release-info.json') -Encoding utf8
    $zip = Join-Path $out "GestureLite-Windows_${Version}_x64-portable.zip"
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
    $archive = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        if ($archive.Entries.Count -ne 5) { throw 'Unexpected portable payload' }
        $entry = $archive.GetEntry('GestureLite.exe')
        $stream = $entry.Open()
        try { $roundtrip = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)).ToLowerInvariant() }
        finally { $stream.Dispose() }
        if ($roundtrip -ne $hash) { throw 'Executable changed during ZIP packaging' }
    } finally { $archive.Dispose() }
    $zipHash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    "$zipHash  $([IO.Path]::GetFileName($zip))" | Set-Content "$zip.sha256" -Encoding ascii
    Write-Output "Portable ZIP verified: $zip"
    Write-Output "Executable SHA256: $hash"
} finally { Pop-Location }
