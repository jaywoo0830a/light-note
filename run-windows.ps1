<#
.SYNOPSIS
    light-note를 윈도우 11에서 점검 → 테스트 → 빌드 → 실행한다.

.DESCRIPTION
    이 스크립트 하나로 윈도우 11에서 필요한 전부를 한다:

        전제조건 점검 → 코어 계약 테스트 → 릴리스 빌드 → 실행

    **전제조건 (스크립트가 직접 확인하고, 빠진 것을 고치는 명령을 알려준다)**
      1. 윈도우 + PowerShell 5.1 이상 (`powershell` 또는 `pwsh`)
      2. rustup + **MSVC** 툴체인 (`x86_64-pc-windows-msvc`)
         — WinUI 3는 MSVC 링커가 필요하다(GNU 툴체인은 링크 불가).
      3. Visual Studio Build Tools의 C++ 도구(`link.exe`)
      4. **Windows App Runtime 2.4** — `windows-reactor` 0.100의 `bootstrap_runtime()`이
         `Microsoft.WindowsAppRuntime.2_8wekyb3d8bbwe` 프레임워크 패키지를 요구한다.
         없으면 앱이 설치 안내 대화상자를 띄우고 `0x8007007E`로 죽는다.

    **리눅스/맥에서는 아무것도 빌드하지 않는다** — WinUI 층은 `cfg(windows)`라
    컴파일 대상이 아니다. 그쪽에서는 `cargo test -p light-note-core`(52개 계약 테스트).

    스크립트 없이 같은 일을 하는 명령(저장소 루트에서):
        cargo test  -p light-note-core
        cargo build -p light-note-win --release
        cargo run   -p light-note-win --release

    참고: 빌드된 exe는 **콘솔 하위 시스템**이라 창과 함께 콘솔도 뜬다(로그 확인용).
    콘솔이 싫으면 `crates/light-note-win/src/main.rs`에
    `#![cfg_attr(windows, windows_subsystem = "windows")]`를 추가한다.

.PARAMETER Prereq
    전제조건만 점검하고 끝낸다(빌드하지 않는다).

.PARAMETER Test
    코어 계약 테스트만 돌린다(`cargo test -p light-note-core`).

.PARAMETER Check
    컴파일만 확인한다(`cargo check --workspace` — WinUI 호스트 포함).

.PARAMETER Build
    빌드만 하고 실행하지 않는다.

.PARAMETER Clean
    `cargo clean` 후 끝낸다(다음 빌드는 전부 다시 컴파일한다).

.PARAMETER DebugBuild
    debug 프로파일로 빌드/실행한다(기본은 `--release`). 별칭 `-Dev`.
    **`-Debug`가 아닌 이유**: `[CmdletBinding()]`이 공통 매개변수로 `-Debug`를
    이미 정의하고 있어서 같은 이름을 쓸 수 없다(스크립트가 죽는다).

.PARAMETER InstallRuntime
    Windows App Runtime이 없으면 `winget`으로 설치를 시도한다.

.PARAMETER OpenDownloads
    Windows App Runtime이 없으면 다운로드 페이지를 브라우저로 연다.

.PARAMETER AppArgs
    앱에 넘길 인자. **`-File`로 실행할 때는 값이 문자열 그대로 전달된다** —
    `-AppArgs "--demo"`는 되지만 `-AppArgs @('-a','-b')`는 배열로 평가되지 않는다
    (PowerShell의 `-File` 규칙). 배열이 필요하면 셸 안에서 직접 부른다:
        & .\run-windows.ps1 -AppArgs @('-a','-b')

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\run-windows.ps1
    powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Prereq
    powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Test
    powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -DebugBuild
    powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Build
    powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Clean
#>
[CmdletBinding()]
param(
    [switch] $Prereq,
    [switch] $Test,
    [switch] $Check,
    [switch] $Build,
    [switch] $Clean,
    [Alias('Dev')]
    [switch] $DebugBuild,
    [switch] $InstallRuntime,
    [switch] $OpenDownloads,
    [string[]] $AppArgs = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# cargo/rustc의 한글 출력이 콘솔에서 깨지지 않게 맞춘다.
try {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    $OutputEncoding = [System.Text.Encoding]::UTF8
} catch {
    # 콘솔이 UTF-8을 거부하면(드묾) 그냥 진행한다.
}

# ── 경로/상수 ────────────────────────────────────────────────────────────────

$root = $PSScriptRoot
if ([string]::IsNullOrEmpty($root)) {
    $root = Split-Path -Parent $MyInvocation.MyCommand.Path
}
$manifest = Join-Path $root 'Cargo.toml'
if (-not (Test-Path -LiteralPath $manifest)) {
    throw "워크스페이스 매니페스트를 찾을 수 없다: $manifest"
}

# windows-reactor 0.100이 요구하는 런타임 (bootstrap.rs의 FRAMEWORK_FAMILY/버전 상수)
$runtimeFamily = 'Microsoft.WindowsAppRuntime.2'
$runtimeMinVersion = [version] '2.4.0.0'
$downloadsUrl = 'https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads'

$profile = if ($DebugBuild) { 'debug' } else { 'release' }
$profileFlag = if ($DebugBuild) { @() } else { @('--release') }
$exeName = 'light-note-win.exe'
$exePath = Join-Path $root "target\$profile\$exeName"

# ── 출력 도우미 ──────────────────────────────────────────────────────────────

function Write-Step {
    param([string] $Text)
    Write-Host ''
    Write-Host "== $Text" -ForegroundColor Cyan
}

function Write-Ok {
    param([string] $Text)
    Write-Host "  [OK]   $Text" -ForegroundColor Green
}

function Write-Bad {
    param([string] $Text)
    Write-Host "  [없음] $Text" -ForegroundColor Red
}

function Write-Warn2 {
    param([string] $Text)
    Write-Host "  [주의] $Text" -ForegroundColor Yellow
}

function Write-Fix {
    param([string] $Text)
    Write-Host "         → $Text" -ForegroundColor DarkGray
}

# ── 전제조건 점검 ────────────────────────────────────────────────────────────

$problems = @()

function Test-Prereq {
    Write-Step '전제조건 점검'

    # 1) 윈도우인가 — WinUI 층은 cfg(windows)라 다른 OS에서는 컴파일 대상이 아니다.
    $onWindows = $true
    if ($PSVersionTable.PSEdition -eq 'Core') { $onWindows = $IsWindows }
    if (-not $onWindows) {
        Write-Warn2 '이 스크립트는 윈도우 전용이다 (WinUI 3 / Windows App Runtime 필요).'
        Write-Warn2 '리눅스/맥에서는 WinUI 호스트가 컴파일되지 않는다.'
        Write-Fix '리눅스/맥: cargo test -p light-note-core   (52개 계약 테스트가 전부 돈다)'
        exit 2
    }
    Write-Ok "윈도우 $([System.Environment]::OSVersion.Version) / PowerShell $($PSVersionTable.PSVersion)"

    # 2) cargo
    if (-not (Get-Command -Name cargo -ErrorAction SilentlyContinue)) {
        Write-Bad 'cargo를 찾을 수 없다'
        Write-Fix 'winget install Rustlang.Rustup   (설치 후 **새 셸**에서 다시 실행)'
        $script:problems += 'cargo 없음'
        return
    }
    Write-Ok "$(& cargo --version)"

    # 3) 툴체인은 MSVC여야 한다 — WinUI 3는 MSVC 링커가 필요하다.
    $hostTriple = ''
    foreach ($line in (& rustc -vV)) {
        if ($line -like 'host:*') { $hostTriple = $line.Substring(5).Trim() }
    }
    if ($hostTriple -like '*-pc-windows-msvc') {
        Write-Ok "툴체인 $hostTriple (MSVC)"
    } else {
        Write-Bad "툴체인이 MSVC가 아니다: $hostTriple"
        Write-Fix 'rustup toolchain install stable-x86_64-pc-windows-msvc'
        Write-Fix 'rustup default stable-x86_64-pc-windows-msvc'
        $script:problems += "툴체인 $hostTriple"
    }

    # 4) MSVC C++ 빌드 도구 (link.exe) — vswhere로 확인한다.
    $pf86 = ${env:ProgramFiles(x86)}
    $vswhere = $null
    if (-not [string]::IsNullOrEmpty($pf86)) {
        $candidate = Join-Path $pf86 'Microsoft Visual Studio\Installer\vswhere.exe'
        if (Test-Path -LiteralPath $candidate) { $vswhere = $candidate }
    }
    $vcTools = 'winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'
    if ($vswhere) {
        $vcRoot = ''
        try {
            $vcRoot = & $vswhere -latest -products * `
                -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
                -property installationPath 2>$null
        } catch {
            $vcRoot = ''
        }
        if ([string]::IsNullOrWhiteSpace($vcRoot)) {
            Write-Bad 'C++ 빌드 도구(MSVC)를 찾지 못했다 — link.exe가 필요하다'
            Write-Fix $vcTools
            $script:problems += 'MSVC 빌드 도구 없음'
        } else {
            Write-Ok "MSVC: $vcRoot"
        }
    } else {
        Write-Warn2 'vswhere.exe가 없다 — Visual Studio / Build Tools가 없을 수 있다'
        Write-Fix $vcTools
    }

    # 5) Windows App Runtime — windows-reactor 0.100의 bootstrap이 요구한다.
    $runtime = @()
    try {
        $runtime = @(Get-AppxPackage -Name "$runtimeFamily*" -ErrorAction Stop)
    } catch {
        $runtime = @()
    }
    if ($runtime.Count -eq 0) {
        Write-Bad "Windows App Runtime $runtimeMinVersion 이상이 없다 ($runtimeFamily)"
        Write-Fix 'winget install --id Microsoft.WindowsAppRuntime.2 --exact --accept-package-agreements --accept-source-agreements'
        Write-Fix "수동 다운로드: $downloadsUrl"
        $script:problems += 'Windows App Runtime 없음'

        if ($OpenDownloads) {
            try {
                Start-Process $downloadsUrl | Out-Null
                Write-Fix '다운로드 페이지를 브라우저로 열었다.'
            } catch {
                Write-Warn2 "브라우저를 열지 못했다: $downloadsUrl"
            }
        }
        if ($InstallRuntime -and (Get-Command -Name winget -ErrorAction SilentlyContinue)) {
            Write-Fix 'winget으로 설치를 시도한다…'
            try {
                & winget install --id Microsoft.WindowsAppRuntime.2 --exact `
                    --accept-package-agreements --accept-source-agreements
                if ($LASTEXITCODE -ne 0) {
                    Write-Warn2 "winget 실패 (exit $LASTEXITCODE) — 수동 설치: $downloadsUrl"
                } else {
                    Write-Ok '설치 요청 완료 — 끝나면 스크립트를 다시 돌려라.'
                }
            } catch {
                Write-Warn2 "winget을 실행하지 못했다 — 수동 설치: $downloadsUrl"
            }
        }
    } else {
        $best = $runtime | Sort-Object -Property Version -Descending | Select-Object -First 1
        if ($best.Version -lt $runtimeMinVersion) {
            Write-Bad "Windows App Runtime $($best.Version) — $runtimeMinVersion 이상이 필요하다"
            Write-Fix "수동 다운로드: $downloadsUrl"
            $script:problems += "런타임 $($best.Version)"
        } else {
            Write-Ok "Windows App Runtime $($best.Version)"
        }
    }
}

# ── cargo 호출 도우미 ────────────────────────────────────────────────────────

function Invoke-Cargo {
    param(
        [Parameter(Mandatory = $true)] [string[]] $CargoArgs,
        # `--` 뒤(= 테스트 하네스)로 넘길 인자. `--manifest-path`는 **반드시**
        # `--` 앞에 있어야 한다 — 뒤에 두면 cargo가 아니라 libtest가 받는다.
        [string[]] $HarnessArgs = @()
    )
    $argv = @($CargoArgs) + @('--manifest-path', $manifest)
    if ($HarnessArgs.Count -gt 0) {
        $argv += @('--') + $HarnessArgs
    }
    Write-Host ('> cargo ' + ($argv -join ' ')) -ForegroundColor DarkGray
    & cargo @argv
    if ($LASTEXITCODE -ne 0) {
        Write-Host "cargo 실패 (exit $LASTEXITCODE)" -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

# ── 본 흐름 ──────────────────────────────────────────────────────────────────

Write-Host ''
Write-Host 'light-note — 윈도우 11 필기 앱 (WinUI 3 / windows-reactor)' -ForegroundColor White
Write-Host "  워크스페이스 : $root"
Write-Host "  프로파일     : $profile"

Test-Prereq

if ($problems.Count -gt 0) {
    Write-Host ''
    Write-Host "전제조건이 빠졌다 ($($problems.Count)개): $($problems -join ', ')" -ForegroundColor Red
    Write-Host '위의 → 명령을 실행한 뒤 다시 돌려라.' -ForegroundColor Red
    exit 1
}

if ($Prereq) {
    Write-Host ''
    Write-Host '전제조건 OK — 빌드할 준비가 됐다.' -ForegroundColor Green
    exit 0
}

if ($Clean) {
    Invoke-Cargo -CargoArgs @('clean')
    Write-Host 'target 정리 완료' -ForegroundColor Green
    exit 0
}

if ($Test) {
    Invoke-Cargo -CargoArgs @('test', '-p', 'light-note-core')
    Write-Host '코어 계약 테스트 통과 (52개)' -ForegroundColor Green
    exit 0
}

if ($Check) {
    Invoke-Cargo -CargoArgs @('check', '--workspace', '--all-targets')
    Write-Host 'cargo check: 컴파일 OK' -ForegroundColor Green
    exit 0
}

# 기본 흐름: 코어 계약 → 빌드 → 실행
Write-Step '코어 계약 테스트 (플랫폼 독립)'
Invoke-Cargo -CargoArgs @('test', '-p', 'light-note-core')

Write-Step "WinUI 호스트 빌드 ($profile)"
Invoke-Cargo -CargoArgs (@('build', '-p', 'light-note-win') + $profileFlag)

if (-not (Test-Path -LiteralPath $exePath)) {
    throw "빌드가 끝났는데 exe가 없다: $exePath"
}
Write-Ok "exe: $exePath"

if ($Build) {
    Write-Host ''
    Write-Host "실행: & '$exePath'" -ForegroundColor Green
    exit 0
}

Write-Step '실행'
Write-Host '  창을 닫으면 스크립트가 끝난다.' -ForegroundColor DarkGray
if ($AppArgs.Count -gt 0) {
    Write-Host "  앱 인자: $($AppArgs -join ' ')" -ForegroundColor DarkGray
}
& $exePath @AppArgs
$code = $LASTEXITCODE
if ($null -eq $code) { $code = 0 }

if ($code -ne 0) {
    Write-Host ''
    Write-Host "앱이 exit $code 로 끝났다." -ForegroundColor Red
    Write-Warn2 'Windows App Runtime 누락(0x8007007E)이면 -Prereq로 확인하고 설치하라.'
    Write-Fix 'winget install --id Microsoft.WindowsAppRuntime.2 --exact'
    Write-Fix "수동 다운로드: $downloadsUrl"
    exit 1
}

Write-Host ''
Write-Host '정상 종료.' -ForegroundColor Green
exit 0
