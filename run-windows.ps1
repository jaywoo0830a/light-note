# light-note — build, run and test on Windows 11.
#
#   .\run-windows.ps1                  # run the app (debug)
#   .\run-windows.ps1 -Release         # run the app (release, no console window)
#   .\run-windows.ps1 -Test            # the core test suite (headless)
#   .\run-windows.ps1 -FetchPdfium     # put pdfium.dll where the app looks for it
#   .\run-windows.ps1 -FetchPdfium -Force   # re-download it even if it is there
#
# Pdfium is bound at **run time** (`pdfium-render`), so the app builds — and the
# whole test suite passes — without the library; only opening a PDF needs it.
# `bind_pdfium()` looks in `crates/light-note/pdfium`, next to the executable, in
# the working directory and in `$env:LIGHT_NOTE_PDFIUM` (that is where this script
# puts the download).
#
# The pen needs the OTD plugin as well, which is a separate install:
#   .\OTD.SharedMemoryOutput\install.ps1 -Enable

[CmdletBinding()]
param(
    # Build and run the app with optimizations (the UI is a real frame loop:
    # debug builds are for debugging, not for judging the feel).
    [switch]$Release,
    # Run `cargo test` instead of the app.  The core is headless, so this works
    # on a machine with no tablet, no window and no Pdfium.
    [switch]$Test,
    # Download pdfium.dll into `crates/light-note/pdfium`.
    [switch]$FetchPdfium,
    # Overwrite an existing pdfium.dll.
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 renders a progress bar per network chunk, which makes a
# 7 MB download take minutes instead of seconds.
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$pdfiumDir = Join-Path $root 'crates\light-note\pdfium'
$pdfiumDll = Join-Path $pdfiumDir 'pdfium.dll'

# Runs cargo from the repository root and fails loudly.
function Invoke-Cargo {
    param([string[]]$CargoArgs)
    Push-Location $root
    try {
        Write-Host "cargo $($CargoArgs -join ' ')"
        & cargo @CargoArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo $($CargoArgs -join ' ') failed ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}

# Downloads the official Pdfium build (bblanchon/pdfium-binaries, x64) and copies
# `bin/pdfium.dll` next to the crate.  Windows 10+ ships bsdtar, so no extra
# tooling is needed to unpack the .tgz.
function Get-Pdfium {
    $url = 'https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-win-x64.tgz'
    $temp = Join-Path ([System.IO.Path]::GetTempPath()) "light-note-pdfium-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Force -Path $temp | Out-Null
    try {
        $archive = Join-Path $temp 'pdfium-win-x64.tgz'
        Write-Host "downloading $url"
        Invoke-WebRequest -Uri $url -OutFile $archive
        & tar -xzf $archive -C $temp
        if ($LASTEXITCODE -ne 0) { throw 'tar could not unpack the archive' }

        $library = Get-ChildItem -Path $temp -Recurse -Filter 'pdfium.dll' | Select-Object -First 1
        if (-not $library) { throw 'the archive does not contain pdfium.dll' }

        New-Item -ItemType Directory -Force -Path $pdfiumDir | Out-Null
        Copy-Item $library.FullName $pdfiumDll -Force
        Write-Host "pdfium.dll -> $pdfiumDll ($([int]($library.Length / 1MB)) MB)"
    } finally {
        Remove-Item $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if ($FetchPdfium) {
    if ((Test-Path $pdfiumDll) -and -not $Force) {
        Write-Host "pdfium.dll is already in place: $pdfiumDll"
        Write-Host 're-download it with:  .\run-windows.ps1 -FetchPdfium -Force'
    } else {
        Get-Pdfium
    }
    if (-not $Test) { return }
}

if ($Test) {
    # The PDF tests skip themselves (loudly) when pdfium.dll is missing.
    Invoke-Cargo -CargoArgs @('test')
    return
}

$otd = Get-Process -Name 'OpenTabletDriver.Daemon' -ErrorAction SilentlyContinue
if (-not $otd) {
    Write-Host @'
note: the OTD daemon is not running, so the app will draw only with the mouse.
      install the plugin with:  .\OTD.SharedMemoryOutput\install.ps1 -Enable
'@
}

if ($Release) {
    Invoke-Cargo -CargoArgs @('run', '--release')
} else {
    Invoke-Cargo -CargoArgs @('run')
}
