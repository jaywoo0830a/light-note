# light-note

윈도우 11에 최적화된 **드로잉패드 필기 앱**. PDF를 배경으로 깔고 그 위에 펜으로 쓰고,
결과를 PNG/PDF로 내보낸다.

- **PDF**: [`hayro`](https://crates.io/crates/hayro) 0.7 — 순수 Rust 해석/래스터화.
  `hayro`가 재수출하는 `vello_cpu`의 `Pixmap`(프리멀티플라이드 RGBA8)을 **배경과 잉크가
  함께 쓴다**. 쓰기는 `pdf-writer`(hayro-write와 같은 writer)라 내보낸 PDF를 다시 읽어
  검증할 수 있다.
- **UI**: [`elm-magic`](https://crates.io/crates/elm-magic) 0.8.6 + `elm-magic-windows-reactor`
  0.8.6 어댑터(선언적 WinUI 3, `windows-reactor` 0.100).

## 구조

```
crates/light-note-core   플랫폼 독립 엔진 — 필기 모델/래스터/PDF 입출력/elm 화면
crates/light-note-win    WinUI 3 호스트 — 윈도우 11 전용 (포인터/파일/표면)
```

| 모듈 | 하는 일 |
|---|---|
| `core::geom` | 좌표계 계약 (pt, **좌상단 원점**) |
| `core::ink` | 도구/스타일/압력 샘플/스트로크 |
| `core::doc` | 페이지·문서·진행 중인 획 |
| `core::history` | **드래그 하나 = Undo 하나** |
| `core::raster` | 스트로크 → 픽셀 (vello_cpu) |
| `core::pdf` | hayro로 열기/크기/래스터화 (읽기 전용) |
| `core::export` | PNG / PDF(벡터 잉크 + 배경 이미지) |
| `core::surface` | WinUI 표면이 무엇을 그릴지 (정적 PNG + 라이브 도형) |
| `core::ui` | elm-magic 화면 + 어댑터 계획(plan) 계약 |

## 테스트 (리눅스에서 전부 돈다)

```bash
cargo test -p light-note-core
```

74개 테스트가 **UI 계약과 인코딩 규칙까지** 검증한다(윈도우에서도 그대로 돈다):

- `ink_model` — 샘플 필터/필압/경계/지우개 히트 테스트
- `document` — Undo/Redo(지우개 드래그 = 편집 하나), 페이지 관리, 취소
- `raster` — 잉크가 **어디에** 올라갔는지 픽셀로 확인, 결정성, PNG 왕복
- `export` — 내보낸 PDF를 **hayro로 다시 읽어** 배경/잉크 위치까지 검증
- `ui_plan` — 버튼 → 의도 매핑, 상태 분기(로딩/빈/준비/실패), `<Raw>` 표면 전달
- `surface` — 정적 레이어가 **언제** 바뀌는가(백그라운드 렌더의 꼬리 규칙, 더블 버퍼의
  스테이징/승격, 빈 레이어 = 1×1 투명 PNG), 그리고 **라이브 도형이 래스터 잉크와 같은
  영역을 덮는가**(픽셀 비교 — 승격 순간에 잉크가 바뀌지 않는 근거)
- `encoding` — `.ps1`의 **UTF-8 BOM**, 모든 텍스트 파일의 UTF-8/CRLF 계약
  (Windows PowerShell 5.1이 BOM 없는 `.ps1`을 ANSI로 읽어 파싱이 죽던 사고의 재발 방지)

## 인코딩 규칙 (윈도우 최적화)

| 파일 | 인코딩 | 줄바꿈 | 이유 |
|---|---|---|---|
| `*.ps1` | **UTF-8 with BOM** | CRLF | Windows PowerShell **5.1**은 BOM 없는 `.ps1`을 **ANSI(CP949)**로 읽는다 |
| `*.rs` `*.toml` `*.md` 등 | UTF-8 **without** BOM | CRLF | rustc/cargo/깃은 항상 UTF-8 — BOM은 이득 없이 diff만 흔든다 |

`.ps1`에 BOM이 없으면 **한글이 깨지는 정도로 끝나지 않는다**: UTF-8 한글 바이트가 CP949의
2바이트 조합으로 재해석되면서 문자열 종결자 `'` 하나가 삼켜지고, "문자열에 종결자가 없습니다"와
`}`/`)` 누락 오류가 캐스케이드로 쏟아진다(실제로 겪은 사고 — 파일 내용은 정상이었다).

`rustfmt.toml`이 `newline_style = "Windows"`를 강제한다 — rustfmt는 기본값으로 파일을
**LF로 다시 써서** 이 계약을 깬다(실제로 겪었고, `encoding` 테스트가 잡아낸다).

사람의 주의력 대신 **기계 세 개**가 지킨다:

- `.editorconfig` — 에디터가 **저장하는 순간** `.ps1`은 `utf-8-bom`, 나머지는 `utf-8`로 쓴다.
- `.gitattributes` — 줄바꿈을 각 PC의 `core.autocrlf`가 아니라 저장소가 정한다(텍스트=CRLF).
- `crates/light-note-core/tests/encoding.rs` — 실제 바이트를 검사한다(BOM·UTF-8·NUL·CRLF).
  `run-windows.ps1`이 **가장 먼저** 돌리는 `cargo test -p light-note-core`에 포함되므로,
  같은 사고는 스크립트를 돌리는 순간 바로 잡힌다.

새 `.ps1`을 만들 때:

```powershell
# VS Code: 우하단 인코딩 표시를 'UTF-8 with BOM'으로 저장
Set-Content -LiteralPath .\new.ps1 -Encoding utf8BOM -Value $text   # 5.1/7 공통
```

- **`.bat`/`.cmd`에는 한글을 쓰지 않는다** — `cmd.exe`는 OEM 코드페이지로 읽고 BOM도 제대로
  다루지 못한다. 필요하면 `.ps1`로 쓴다.
- `working-tree-encoding=UTF-8-BOM` 같은 git 속성은 **쓰지 않는다**(깃 문서가 BOM 처리 문제로
  권장하지 않는다) — BOM은 파일 안의 실제 바이트로 둔다.
- 콘솔에서 한글이 깨지면 `chcp 65001` — 스크립트는 스스로 `[Console]::OutputEncoding`을
  UTF-8로 맞춘다.

## 실행 (윈도우 11)

```powershell
# 전제조건 점검 → 코어 계약 테스트 → 릴리스 빌드 → 실행 (한 번에)
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1

# 전제조건만 / 테스트만 / 컴파일만 / 빌드만
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Prereq
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Test
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Check
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Build
```

스크립트 없이 직접:

```powershell
cargo test  -p light-note-core
cargo run   -p light-note-win --release
```

### 전제조건 (`run-windows.ps1`이 직접 확인한다)

| 항목 | 없을 때 |
|---|---|
| PowerShell 5.1+ (윈도우) | — |
| rustup + **MSVC** 툴체인 (`x86_64-pc-windows-msvc`) | `rustup toolchain install stable-x86_64-pc-windows-msvc` |
| Visual Studio Build Tools (C++ 도구, `link.exe`) | `winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| **Windows App Runtime 2.4** (`Microsoft.WindowsAppRuntime.2_8wekyb3d8bbwe`) | `-InstallRuntime`(winget) 또는 `-OpenDownloads` — [다운로드](https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads) |

마지막 항목은 `windows-reactor` 0.100의 `bootstrap_runtime()`이 요구하는 것이다
(`src/native/winui/bootstrap.rs`의 `FRAMEWORK_FAMILY` / 버전 상수 = 2.4). 없으면 앱이
설치 안내 대화상자를 띄우고 `0x8007007E`로 죽는다 — 스크립트가 미리 잡아 준다.

## Windows 11 최적화 포인트

1. **확정 레이어 = PNG 한 장** — 드래그가 *끝날 때만* 다시 만든다(`InkSurface` 계약).
2. **그 PNG는 백그라운드에서 만든다** — A4 한 장이 13ms(release)/277ms(dev)이고 그 80%가
   PNG 인코딩이다. 포인터 메시지 처리 안에서 돌리면 **획을 끝낼 때마다 화면이 멈춘다**.
   그래서 UI 스레드는 워커에 맡기고, 도착 전까지 방금 확정한 획을 라이브 도형으로 그려
   빈 틈을 메운다(`surface::pending_ink`). 낡은 렌더 결과는 **세대 번호**로 버리고,
   렌더 중에 또 그으면 작업을 다음 한 번으로 합친다(한 번에 하나만 돈다).
3. **정적 레이어는 두 장(더블 버퍼) — 깜빡임이 구조적으로 없다** —
   `Image.Source`를 바꾸면 어댑터(`windows-reactor` 0.100)는 **먼저 소스를 비우고** 그 다음
   비동기로 디코드한다(`SetSourceAsync` → 끝나면 `SetSource` + `ImageOpened`). 그래서
   한 장만 쓰면 그 사이 잉크가 **통째로 사라진다**(매 획마다 깜빡임).
   지금은 새 PNG를 **창 밖으로 밀어 둔 뒤 레이어**에 올리고, 디코드가 끝났다는
   신호를 받은 뒤에만 앞뒤를 바꾼다(`surface::StaticLayers`). 신호가 끝내 오지 않아도
   화면이 멈추지 않도록 **150ms 안전망**을 함께 건다(그 사이 화면은 옛 레이어 + 라이브 꼬리).
   화면의 잉크는 **한 프레임도 비지 않는다**.
4. **라이브 도형은 래스터와 *같은 도형*이다 — 승격 순간에 모양이 바뀌지 않는다** —
   라이브 레이어는 WinUI `Line`(직선)으로만 그릴 수 있는데, 래스터의 곡선과 다른 도형을
   쓰면 획을 확정하는 순간(라이브 → PNG 승격) 잉크가 "딱" 바뀐다. 그래서 도형 결정을
   [`raster::ink_shape`] **하나**로 모으고, 래스터도 라이브와 **같은 폴리라인**으로 채운다:
   - 곡선(폭 일정) → 같은 중점 2차 베지어를 같은 오차(0.25px)로 편다(`raster::flatten_spans`).
   - 폭 가변 → 구간 사각형을 **반지름만큼 늘려** 관절의 둥근 조인을 덮는다
     (`raster::extended_span` — 원은 변이 `r`인 정사각형에 내접한다 → 도형을 더 그릴 필요가 없다).
   - 양 끝은 **둥근 캡** — WinUI `Line`에는 캡 속성이 없어서 채운 `Ellipse`로 그린다.
   테스트가 **픽셀로** 확인한다: 라이브 도형과 래스터 잉크의 차이 = **0.000%**(6가지 획).
5. **한 획 = 한 번의 채움 = 한 합성 그룹** — 래스터는 한 획을 `fill_path` **한 번**으로
   채우므로 겹친 부분(관절·캡)이 두 번 곱해지지 않는다. 라이브 레이어도 자식 도형을
   불투명하게 그리고 **그룹 불투명도**(`Canvas.Opacity`)를 준다 — WinUI가 자식들을 먼저
   합성한 뒤 한 번만 투명도를 적용하므로 **형광펜이 얼룩지지 않는다**(예전에는 구간마다
   따로 스트로크해서 관절마다 알파가 90 → 149로 진해졌다).
6. **포인터는 `Border`** — `windows-reactor` 0.100에서 포인터 이벤트는 `Border`에만 있다.
   `Canvas`는 절대 좌표 배치, `Image`는 정적 레이어 전용이다.
7. **입력은 메시지 큐를 거쳐** 처리된다(Reactor의 이벤트 FIFO) — elm의 "이벤트 = 할당"과 맞는다.

비용은 **예제로 잰다** — 숫자를 바꾸기 전에 먼저 재라:

```bash
cargo run --release -p light-note-core --example surface_cost
```

정적 레이어(획 수별) / 그 안쪽(래스터·픽셀 스캔·PNG 인코딩) / 라이브 도형 / 뷰모델 생성
시간을 한 번에 출력한다. **`-DebugBuild`(opt-level 0)는 20배 느리다** — 필기감을 볼 때는
기본(릴리스)로 돌려라.

## 알아둘 한계 (어댑터/런타임 사실)

- **`Image.Source` 교체는 비동기다**: 어댑터는 소스를 **먼저 비우고**(`clear_property`)
  `SetSourceAsync`로 디코드한 뒤 완료되면 붙이고 `ImageOpened`를 올린다. 그래서 표면은
  **두 장(더블 버퍼)** 이고, 새 그림은 창 밖에 밀어 둔 뒤 레이어에서 디코드가 끝난 뒤에만
  승격된다(`surface::StaticLayers`). 소스가 바뀐 레이어가 화면에 노출되는 경로가 없어야
  깜빡임이 사라진다 — 이 규칙을 깨면 매 획마다 잉크가 번쩍인다.
- **WinUI `Line`에는 캡/조인 속성이 없다**: `windows-reactor` 0.100이 노출하는 `Line` 속성은
  `Stroke`/`StrokeThickness`/`X1..Y2`뿐이다(`Path`·`Polyline`은 아예 없다). 그래서 둥근 캡은
  **채운 `Ellipse`**, 관절의 둥근 조인은 **구간 사각형을 반지름만큼 늘려** 흉내낸다
  (`raster::extended_span` — 원은 변이 `r`인 정사각형에 내접하므로 늘린 사각형이 관절을 덮는다).
  대신 관절 모서리가 `≈0.15r²`만큼 더 채워지는데, 눈에 띄지 않고 **래스터/PNG/PDF와 라이브가
  같은 도형**이 되는 이득이 훨씬 크다(픽셀 차이 0.000%).
- **겹쳐 그리면 반투명 색이 진해진다**: 래스터는 한 획을 한 번의 채움으로 그리므로, 라이브
  레이어도 **한 획 = 한 합성 그룹**(`Canvas.Opacity` = 색의 알파, 자식 도형은 불투명)으로
  그려야 같은 색이 나온다. 이 규칙을 깨면 형광펜의 캡/관절만 진해진다.
- **PDF 내보내기는 폭이 일정한 획을 진짜 베지어 곡선으로 긋는다**(화면 PNG는 라이브 레이어와
  같은 폴리라인 — 오차 0.25px). 벡터 문서에서는 곡선이 더 낫기 때문이고, 확대해도 각지지 않는다.
  폭이 변하는 획은 화면과 **같은 합집합**(사각형 + 둥근 캡)을 한 번 채운다(관절 얼룩 없음).

- **필압**: `PointerEventInfo`에 압력이 없어 **속도로 굵기를 만든다**(`pressure_from_speed`).
  펜 태블릿 압력은 어댑터가 이벤트를 확장해야 한다.
- **단축키**: `AcceleratorKey`가 `R`, NumPad, `Add`/`Subtract`, `Enter`만 지원한다 →
  `Ctrl+더하기/빼기`(줌), `Ctrl+Enter`(페이지 추가)만 붙였다. `Ctrl+Z` 등은 어댑터가
  `ElmView` 핸들을 노출하면 `drive::dispatch_key`로 연결할 수 있다. 그래서 같은 동작을
  **툴바 버튼**으로도 제공한다.
- **`<Raw>` 클로저는 슬롯/props를 못 본다**(토큰이 그대로 삽입된다) — 표면 재료는 호스트가
  `ui::stage_surface`로 넘기고 클로저가 꺼내 쓴다.
- **콜백 prop은 렌더당 한 번만** 부를 수 있다(매크로가 `move` 클로저로 감싼다) →
  버튼마다 prop을 하나씩 둔다.
- **자식은 슬롯을 직접 쓰지 않는다** — 공유 값은 호스트가 소유하고 props + 콜백으로 내린다.

