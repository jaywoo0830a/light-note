# OTD.SharedMemoryOutput 설치 스크립트
#
# 하는 일 (순서가 중요하다):
#   1. dotnet SDK가 있는지 확인한다 (없으면 멈춘다 — 빌드가 불가능하다)
#   2. Release로 빌드한다
#   3. dll을 OTD 플러그인 폴더로 복사한다  %LOCALAPPDATA%\OpenTabletDriver\Plugins\OTD.SharedMemoryOutput\
#   4. (선택) settings.json에 필터를 등록한다 — **데몬이 꺼져 있을 때만**
#
# 사용법:
#   .\install.ps1              # 빌드 + 복사만
#   .\install.ps1 -Enable      # + settings.json에 필터 등록
#
# 되돌리기: settings.json.bak-* 파일을 원래 이름으로 되돌리고 플러그인 폴더를 지운다.

[CmdletBinding()]
param(
    [switch]$Enable
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$pluginDir = Join-Path $env:LOCALAPPDATA 'OpenTabletDriver\Plugins\OTD.SharedMemoryOutput'
$settingsPath = Join-Path $env:LOCALAPPDATA 'OpenTabletDriver\settings.json'
$filterPath = 'OTD.SharedMemoryOutput.SharedMemoryTap'

# ── 1. SDK 확인 ───────────────────────────────────────────────────────────
# PATH의 `dotnet`이 SDK를 못 찾을 때가 있다(런타임만 있는 설치, PATH 우선순위 등).
# 그래서 PATH → Program Files → 사용자 로컬 순으로 확인한다.
$dotnet = (Get-Command dotnet -ErrorAction SilentlyContinue | Select-Object -First 1).Source
$sdk = if ($dotnet) { ((& $dotnet --list-sdks 2>$null) -join "`n") } else { '' }
if ([string]::IsNullOrWhiteSpace($sdk)) {
    $dotnet = @(
        (Join-Path $env:ProgramFiles 'dotnet\dotnet.exe'),
        (Join-Path $env:USERPROFILE '.dotnet\dotnet.exe')
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($dotnet) { $sdk = ((& $dotnet --list-sdks 2>$null) -join "`n") }
}

if ([string]::IsNullOrWhiteSpace($sdk)) {
    Write-Error @"
.NET SDK가 없습니다. 런타임만으로는 C#을 컴파일할 수 없습니다.
    winget install Microsoft.DotNet.SDK.8
설치 후 새 터미널에서 다시 실행하세요. (OTD 0.6.7은 .NET 8로 돕니다)
"@
}
Write-Host "dotnet: $dotnet"
Write-Host "SDK: $($sdk.Trim())"

# ── 2. 빌드 ───────────────────────────────────────────────────────────────
Push-Location $here
try {
    & $dotnet build -c Release --nologo
    if ($LASTEXITCODE -ne 0) { throw "빌드 실패" }
} finally {
    Pop-Location
}

$out = Join-Path $here 'bin\Release'
if (-not (Test-Path $out)) { throw "빌드 산출물이 없습니다: $out" }

# ── 3. 복사 ───────────────────────────────────────────────────────────────
New-Item -ItemType Directory -Force -Path $pluginDir | Out-Null
Get-ChildItem $out -File | Where-Object { $_.Extension -in '.dll', '.json' } |
    Where-Object { $_.Name -ne 'OpenTabletDriver.Plugin.dll' } |
    Copy-Item -Destination $pluginDir -Force
Write-Host "복사 완료: $pluginDir"
Get-ChildItem $pluginDir | Select-Object Name, Length | Format-Table -AutoSize

if (-not $Enable) {
    Write-Host @"

플러그인 파일만 복사했습니다. OTD에서 쓰려면:
  - OTD GUI의 Filters 탭에서 'Shared Memory Tap (light-note)'를 켜거나
  - 이 스크립트를 -Enable로 다시 실행하세요 (settings.json에 등록)
"@
    return
}

# ── 4. settings.json에 필터 등록 ──────────────────────────────────────────
$daemon = Get-Process -Name 'OpenTabletDriver.Daemon' -ErrorAction SilentlyContinue
if ($daemon) {
    Write-Error @"
데몬이 돌고 있습니다(pid $($daemon.Id)). 데몬은 종료할 때 settings.json을 다시 쓰기 때문에
지금 고치면 덮어써집니다. 데몬을 끄고 다시 실행하세요.
"@
}

$backup = "$settingsPath.bak-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
Copy-Item $settingsPath $backup -Force
Write-Host "백업: $backup"

$settings = Get-Content $settingsPath -Raw | ConvertFrom-Json
$filters = @($settings.Profiles[0].Filters)
if ($filters.Path -contains $filterPath) {
    Write-Host "이미 등록되어 있습니다: $filterPath"
} else {
    $entry = [pscustomobject]@{ Path = $filterPath; Settings = @(); Enable = $true }
    $settings.Profiles[0].Filters = @($filters) + $entry
    $settings | ConvertTo-Json -Depth 32 | Set-Content $settingsPath -Encoding utf8
    Write-Host "등록: $filterPath (Enable = true)"
}

Write-Host @"

이제 OTD 데몬을 다시 시작하세요:
  & 'C:\Users\rlawjddn\Downloads\OpenTabletDriver-0.6.7_win-x64\OpenTabletDriver-0.6.7_win-x64\OpenTabletDriver.Daemon.exe'
확인: 펜을 태블릿에 대고 light-note를 실행 — 상태줄에 'OTD 공유메모리'가 보여야 합니다.
"@
