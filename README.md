# light-note

윈도우 11에 최적화된 **드로잉패드 필기 앱**. PDF를 배경으로 깔고 그 위에 펜으로 쓰고,
결과를 PNG/PDF로 내보낸다.

- **PDF**: [`hayro`](https://crates.io/crates/hayro) 0.7 — 순수 Rust 해석/래스터화.
  `hayro`가 재수출하는 `vello_cpu`의 `Pixmap`(프리멀티플라이드 RGBA8)을 **배경과 잉크가
  함께 쓴다**. 쓰기는 `pdf-writer`(hayro-write와 같은 writer)라 내보낸 PDF를 다시 읽어
  검증할 수 있다.
- **UI**: [`elm-magic`](https://crates.io/crates/elm-magic) 0.8.7 + `elm-magic-windows-reactor`
  0.8.7 어댑터(선언적 WinUI 3, `windows-reactor` 0.100). 파일 대화상자는 `rfd` 0.15.
- **입력**: Win32 `WM_POINTER`(`windows` 0.62) — **디지타이저(펜)만 필기**하게 하고
  필압·틸트를 여기서 읽는다(`digitizer`). WinUI의 `PointerEventInfo`에는 장치도, pointer id도,
  압력도 없기 때문이다.

## 구조 — 크레이트 하나, 파이프라인 하나

```
crates/light-note-gui    윈도우 11 전용 앱 (WinUI 3 호스트 + 엔진)
```

엔진과 호스트를 나누지 **않는다**: 좌표·기하·문서·표면이 전부 같은 `Pixmap`과 같은 도형
결정을 공유하는데, 크레이트를 가르면 그 계약이 `pub` 경계를 넘느라 문장이 길어지고
(테스트도 두 곳으로 갈라지고) 경계를 넘는 값이 실제로 늘었다. 지금은 **모듈**로 나누고
`cargo test`가 WinUI 없이 돈다(WinUI 타입은 `render`/`app`/`files`에만 있다).

| 모듈 | 단계 | 하는 일 |
|---|---|---|
| `geom` | — | 좌표계 계약(pt, **좌상단 원점**), 배율(pt↔px), 선분 거리 |
| `ink` | — | 도구·스타일·압력 표본·스트로크(샘플 필터, 히트 테스트) |
| `doc` | — | 페이지·문서·편집 기록(Undo/Redo) — **진행 중 획은 여기 없다** |
| `input` | ① | WinUI 포인터(DIP) → 표본(`Pt` + 위상 + **장치·필압·틸트**), O(1) |
| `digitizer` | ① | Win32 `WM_POINTER` → 장치·필압·틸트(창 서브클래스, **관찰만**) — Windows 전용 |
| `tool` | ② | 표본 → 획·지우개, press/drag/lift/cancel, **펜만 필기**, 압력(필압, 없으면 속도) |
| `canvas` | ③ | 문서 + **구운 접두사** + **안 구운 꼬리**, 베이크 요청(latest-wins) |
| `render` | ④ | `Frame` → WinUI 트리(키 diff), 베이크(워커), 가속키 |
| `ui` | ④ | elm-magic 화면(`Screen` + 조각 슬롯 `<Raw>`) + `<Raw>` 경계, **전부 영어** |
| `style` | ④ | **디자인 토큰** — 반지름·간격·글자 크기(플랫폼 무관, 테스트 대상) |
| `parts` | ④ | 화면 조각 — **조각마다 파일 하나**: 머리글·미리보기·페이지 목록·상태바·종이 |
| `shape` | ④ | **도형 하나를 정하는 곳** — 래스터·라이브·PDF가 모두 여기를 쓴다 |
| `pdf` | 밖 | hayro로 PDF 열기/크기/래스터화(읽기 전용) |
| `export` | 밖 | PNG / PDF(벡터 잉크 + 배경 이미지) |
| `files` | 밖 | 네이티브 파일 대화상자(IFileDialog) |
| `app` | 셸 | 4단계를 **순서대로 부르는 유일한 곳**(elm `Component`, 워커) |

## 스타일과 언어 (windows-reactor 예제 21~30의 결론)

이 백엔드는 `css!`/`class`를 읽지 않는다. 스타일을 만드는 통로는 **`<Raw>` 하나**이고,
`<Raw>` 본문은 매크로가 그대로 복사하므로 그 안에서 elm 상태를 읽을 수도 없다. 그래서:

- **화면(`ui.rs`)에는 버튼도 색도 간격도 없다** — 조각을 선언하고(`<Header />` 등) 값을
  내려보낸다. **elm의 `<Button>`은 쓰지 않는다**: WinUI 기본 모양으로 굳어 아이콘·크기·
  색·툴팁을 줄 방법이 없다.
- **버튼은 조각이 만든다**(`parts/buttons.rs`). `<Raw>` 안에서 elm 콜백은 못 쓰지만
  **호스트가 넘긴 `IntentSink`는 캡처할 수 있다** — 표면이 포인터 이벤트를 다루는 것과
  같은 방법이다(`app.rs`가 `sender`를 캡처해 통로를 등록한다). 그래서 버튼 하나는
  `Button::new().on_click(|| sink(intent))`이고, 아이콘은 WinUI 내장 `SymbolIcon`,
  활성 도구는 `ButtonStyle::Accent`, 나머지는 `Subtle`(호버에만 배경)이며
  **툴팁 + 자동화 이름을 항상 같이** 준다(아이콘만 있는 버튼의 접근성).
- **조각마다 파일 하나 + 함수 하나**(`parts/`) — 앱바 · 툴바(버튼 + 잉크 미리보기) · 레일 ·
  상태바 · 빈 상태 · 여는 중 · 실패 · 단축키 · 종이.
- **정의는 플랫폼 무관**(`ui.rs`) — `Intent::TOOLBAR`/`Intent::RAIL`/`Intent::CHROME`가
  "어떤 의도가 어디에 있는가"를 들고 있고 `tests/ui_plan.rs`가 빠짐·중복·라벨을 검증한다.
  그리는 일만 WinUI가 한다.
- **숫자와 색은 토큰 하나**(`style.rs`) — 간격은 4 DIP의 배수(4/8/12/16/24), 글자 크기는
  내림차순 4단계, 색은 **테마 브러시 이름**이라 라이트/다크/고대비가 공짜로 따라온다.
  규칙은 `tests/style.rs`가 고정한다.
- 창 자체의 스타일(테마·Mica 배경)은 호스트만 만질 수 있다(`app.rs`의 `WindowVisuals`).

### 레이아웃 (조각의 자리)

```text
Grid (루트: 가속기 Ctrl+±/Ctrl+Enter)                        app.rs
└ 앱바      제목 · 배경(PDF) · 배지(페이지/배율/입력/저장 안 됨)   parts/header.rs
├ 툴바      아이콘 버튼 14개(5묶음) + 잉크 미리보기(색·굵기)      parts/toolbar.rs
├ 정보 띠   상태 문구 · 힌트 · 도구/획/파이프라인/배율            parts/status.rs
├ 본문 ┬ 레일  페이지 목록(줄을 누르면 이동) · 조작 · 배율        parts/rail.rs
│      └ 종이  잉크 표면 (빈 상태 안내 / 스피너 / 실패 카드)      render.rs · parts/{paper,empty,loading,failure}.rs
└ 단축키    F1로 열고 Esc로 닫는다(호스트가 상태를 소유)          parts/shortcuts.rs
```

### 이 백엔드에서 **믿을 수 있는 것 / 없는 것** (실측으로 확인)

| 쓰고 싶은 것 | 이 버전의 현실 |
|---|---|
| `StackPanel`(elm의 `<Col>`/`<Row>`) | ✅ 자식 전부 마운트되고 순서대로 배치된다 |
| `width`/`height`/`opacity`/`margin` | ✅ `LayoutControl` 기본 속성은 내려간다 |
| `HorizontalAlignment`/`VerticalAlignment` | ✅ 내려간다(단, 부모가 자식에게 폭을 줘야 의미가 있다) |
| `Grid::columns()`(열 정의) + `Grid.Column` | ❌ 첫 열이 남은 폭을 다 먹어(무한 폭으로 측정) **둘째 열의 자식이 화면 밖으로** 나간다 |
| `Grid` 한 셀 + 오른쪽 정렬 | ❌ 셀 폭이 0이라 자식이 **왼쪽 바깥**으로 밀린다 |
| `RelativePanel` 정렬 attached property | ❌ 자식이 제자리에 남는다 |
| `ScrollViewer`의 높이 | ⚠️ 창 크기(`on_window_size`)를 내려주면 스크롤이 생긴다 — **추정값**(`TOKENS.chrome_h`)에 기댄다 |

그래서 배치는 **줄을 나누는 것**으로만 하고(양 끝 정렬 대신 두 줄), 잉크 영역 높이는
호스트가 창 크기를 관측해 내려주며, 정보 띠는 **본문 위**에 둔다(추정이 틀려도 잘리는 것은
종이의 아래쪽뿐이다).

**사용자가 읽는 문자열은 전부 영어다**(버튼·상태 문구·힌트·오류·배지).
`tests/ui_plan.rs::every_visible_string_is_english_only`가 비-ASCII 알파벳을 금지한다
(`—`/`·`/`…` 같은 구두점은 허용).

**폰트 — `Google Sans Flex`는 지금 적용할 수 없다**:
Google Fonts의 `<link rel="stylesheet">`는 HTML/CSS 기법이라 네이티브 WinUI에는 넣을 자리가
없고, WinUI에서 글꼴을 정하는 `TextBlock.FontFamily`를 `windows-reactor` 0.100이 **노출하지
않는다**(`font_size`/`font_weight`/`foreground`/`text_wrapping`/`max_lines`/`text_trimming`만
있다 — 예제 22의 결론). `ResourceOverrides`도 `Color`/`Thickness`/`CornerRadius`만 받는다.
이름은 `style::Tokens::FONT_FAMILY`에 두었으니 업스트림이 `font_family`를 열면 그 한 곳만 쓰면
된다. 지금은 WinUI 기본 글꼴(Windows 11의 `Segoe UI Variable`)로 두고, 타이포는
**크기 · 굵기 · 색 · 자름** 네 축으로 만든다.

## 4단계 파이프라인

```text
① HardwareInput  [UI]    WinUI 포인터 + WM_POINTER → Sample   (input, digitizer)
② CanvasTool     [UI]    Sample → 획 + 라이브 기하            (tool)
③ Canvas         [UI]    상태만: 획 목록 · 구운 접두사 · 꼬리   (canvas)
④ Render         [UI]    Canvas → WinUI 트리(키 diff)        (render)
                 [워커]  Canvas 스냅샷 → 픽스맵 + PNG          ← 페이지 크기 작업은 여기서만
```

규칙은 셋뿐이고, 그 셋이 이 앱의 설계 전부다:

- **R1 — UI 단계는 페이지 크기에 비례하는 일을 하지 않는다.**
  표본 하나는 O(1)이고, ④-UI가 만드는 도형 수는 `canvas::LIVE_SHAPE_BUDGET`(900)으로 묶인다.
  예산을 넘으면 ③이 **베이크를 요청**한다. (측정: 600점 획의 라이브 기하는 **0.1ms**.)
- **R2 — 픽셀 작업은 ④-워커에서만, 그리고 기다리지 않는다.**
  요청은 **최신 하나**만 의미가 있다(`Canvas::last_request`, latest-wins).
  UI 스레드는 결과를 기다리며 멈추지 않는다 — 늦게 온 응답은 **버린다**.
- **R3 — 화면은 언제나 "구운 접두사 + 안 구운 꼬리"로 완전하다.**
  접두사가 낡아도(지우개·되돌리기·페이지 이동) 꼬리가 그 획들을 그리므로 화면이 비지 않는다.
  그래서 **타이머도, 세대 장부도, 안전망 스레드도 없다** — 예전 설계에 있던 그 셋을
  규칙 하나가 대신한다.

단계별 코드 경로·비용·불변식은 [docs/stroke-pipeline.md](docs/stroke-pipeline.md)에 정리했다.

## 테스트 (WinUI 없이 전부 돈다)

```bash
cargo test -p light-note-gui
```

70개 테스트가 **화면 계약과 인코딩 규칙까지** 검증한다:

- `pipeline` — ①표본 정규화(표면 DIP → pt)·**펜 프레임**(장치·필압·틸트, 낡은 프레임은 버림)
  ②드래그 하나 = 편집 하나·취소는 흔적 없음·**펜만 필기**(마우스·손가락은 무시)·**필압이
  속도를 이긴다** ③꼬리는 **구운 접두사 뒤에서 시작**·낡은 응답은 버려짐·예산이 상한 ④화면 계획
- `geometry` — **라이브 도형과 래스터가 같은 픽셀인가**(곡선/가변폭/점/형광펜 4종, 바이트
  비교), 관절을 덮는 확장 규칙, 관절에서 알파가 두 번 곱해지지 않는가
- `document` — Undo/Redo(지우개 드래그 = 편집 하나, 페이지 편집도 한 편집), 히스토리 한계,
  지우개 판정(선분 거리 + 획 반폭)
- `export` — 내보낸 PDF를 **hayro로 다시 읽어** 페이지 수/크기를 검증, PNG 배경 합성,
  파일 이름 규칙, 배경 페이지가 없으면 **조용히 넘어가지 않고 실패**
- `ui_plan` — 버튼 18개 → 의도 매핑, 상태 분기(빈/준비/여는 중/실패), 상태바, `<Raw>` 경계가
  **정확히 하나**이고 등록된 빌더에 재료가 그대로 도착하는가
- `encoding` — `.ps1`의 **UTF-8 BOM**, 모든 텍스트 파일의 UTF-8/CRLF 계약
  (Windows PowerShell 5.1이 BOM 없는 `.ps1`을 ANSI로 읽어 파싱이 죽던 사고의 재발 방지)

## 계측 — 숫자를 바꾸기 전에 먼저 재라

```bash
cargo run --release -p light-note-gui --example bake_cost
```

A4(893×1263px, 1.13Mpx) 기준 release 실측(개발 PC):

| 재는 것 | 값 |
|---|---|
| 베이크 1획 (래스터+PNG) | **11.5ms** |
| 베이크 20획 / 300획 | 14.0ms / **28.8ms** |
| 그 안쪽: 잉크 래스터 / **PNG 인코딩** | 2.1ms / **14.3ms** |
| 라이브 기하 600점 (UI 스레드) | **0.1ms** |
| `Canvas::frame` / `Frame` 복사 (매 프레임) | **0.0ms**(`Rc` 복사) |

읽는 법: **베이크의 80%가 PNG 인코딩**이다(A4 전체를 도는 비용) → 그래서 워커로 뺐고(R2),
UI가 하는 일은 꼬리 도형 몇 개뿐이다(R1). `-DebugBuild`(opt-level 0)는 20배 느리다 —
필기감을 판단할 때는 `--release`로 돌려라.

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
- `crates/light-note-gui/tests/encoding.rs` — 실제 바이트를 검사한다(BOM·UTF-8·NUL·CRLF).
  `run-windows.ps1`이 **가장 먼저** 돌리는 `cargo test -p light-note-gui`에 포함되므로,
  같은 사고는 스크립트를 돌리는 순간 바로 잡힌다. (`Cargo.lock`만 예외다 — cargo가 소유하고
  `\n`으로 다시 쓰므로 우리 줄바꿈 계약의 대상이 아니다.)

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
# 전제조건 점검 → 계약 테스트 → 릴리스 빌드 → 실행 (한 번에)
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1

# 전제조건만 / 테스트만 / 컴파일만 / 빌드만
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Prereq
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Test
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Check
powershell -ExecutionPolicy Bypass -File .\run-windows.ps1 -Build
```

스크립트 없이 직접:

```powershell
cargo test  -p light-note-gui
cargo run   -p light-note-gui --release
```

### 전제조건 (`run-windows.ps1`이 직접 확인한다)

| 항목 | 없을 때 |
|---|---|
| PowerShell 5.1+ (윈도우) | — |
| rustup + **MSVC** 툴체인 (`x86_64-pc-windows-msvc`) | `rustup toolchain install stable-x86_64-pc-windows-msvc` |
| Visual Studio Build Tools (C++ 도구, `link.exe`) | `winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| **Windows App Runtime 2.4** (`Microsoft.WindowsAppRuntime.2_8wekyb3d8bbwe`) | `-InstallRuntime`(winget) 또는 `-OpenDownloads` — [다운로드](https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads) |

마지막 항목은 `windows-reactor` 0.100의 `bootstrap_runtime()`이 요구하는 것이다. 없으면 앱이
설치 안내 대화상자를 띄우고 `0x8007007E`로 죽는다 — 스크립트가 미리 잡아 준다.

## Windows 11 최적화 포인트

1. **추가는 베이크를 부르지 않는다** — 획 하나를 확정하는 일(`commit`)은 접두사를 낡게 하지
   않는다. 그 획은 **꼬리**로 들어가고, 화면은 키가 바뀐 그룹 하나만 다시 그린다(키 diff).
   지우개·되돌리기·페이지 이동처럼 **픽셀에서 뺄 수 없는 편집**만 베이크를 부른다(R3).
2. **그 베이크는 워커에서 돈다** — A4 한 장 베이크는 11.5ms(1획)~28.8ms(300획)이고 그 80%가
   **PNG 인코딩**이다. 포인터 메시지 처리 안에서 돌리면 화면이 멈춘다 — 그래서 UI 스레드는
   워커에 맡기고, 도착 전까지 꼬리가 그 획들을 그려 빈 틈을 메운다. 워커에 가 있는 요청은
   **최신 하나만** 의미가 있고(`last_request`), 낡은 응답은 조용히 버려진다(R2).
3. **베이스는 두 장(더블 버퍼) — 깜빡임이 구조적으로 없다** — `Image.Source`를 바꾸면
   어댑터(`windows-reactor` 0.100)는 **먼저 소스를 비우고** 그다음 비동기로 디코드한다
   (`SetSourceAsync` → 끝나면 `SetSource` + `ImageOpened`). 그래서 한 장만 쓰면 그 사이 잉크가
   **통째로 사라진다**. 지금은 새 PNG를 **창 밖으로 밀어 둔 뒤 자리**에 넣고, 디코드 완료
   신호를 받은 뒤에만 앞뒤를 맞바꾼다(`Base::stage`/`promote`). 각 자리가 **자기 페이지·배율·
   획 수**를 기억하므로, 그 사이 페이지가 바뀌면 대기 중 그림을 그냥 버린다. 예전에 있던
   **150ms 안전망 타이머는 없다** — 꼬리가 항상 완전하므로(R3) 타이머가 메울 틈이 없다.
4. **라이브 도형은 래스터와 *같은 도형*이다 — 확정 순간에 모양이 바뀌지 않는다** —
   라이브 레이어는 WinUI `Line`(직선)으로만 그릴 수 있는데, 래스터의 곡선과 다른 도형을 쓰면
   획을 확정하는 순간(라이브 → PNG 승격) 잉크가 "딱" 바뀐다. 그래서 도형 결정을
   `shape::ink_shape` **하나**로 모으고, 래스터도 라이브와 **같은 폴리라인**으로 채운다:
   - 곡선(폭 일정) → 같은 중점 2차 베지어를 같은 오차(0.25px)로 편다(`shape::flatten_spans`).
   - 폭 가변 → 구간 사각형을 **반지름만큼 늘려** 관절의 둥근 조인을 덮는다
     (`shape::extended_span` — 원은 변이 `r`인 정사각형에 내접한다 → 도형을 더 그릴 필요가 없다).
   - 양 끝은 **둥근 캡** — WinUI `Line`에는 캡 속성이 없어서 채운 `Ellipse`로 그린다.
   - 태블릿 없이 점 하나를 찍으면 **점 하나**(원) — 래스터도 같은 원 하나를 채운다.
   `geometry` 테스트가 **픽셀로** 확인한다: 라이브 도형과 래스터 잉크의 바이트 차이 = **0**.
5. **한 획 = 한 번의 채움 = 한 합성 그룹** — 래스터는 한 획을 `fill_path` **한 번**으로
   채우므로 겹친 부분(관절·캡)이 두 번 곱해지지 않는다. 라이브 레이어도 자식 도형을
   불투명하게 그리고 **그룹 불투명도**(`Canvas.Opacity`)를 준다 — WinUI가 자식들을 먼저
   합성한 뒤 한 번만 투명도를 적용하므로 **형광펜이 얼룩지지 않는다**(예전에는 구간마다
   따로 스트로크해서 관절마다 알파가 90 → 149로 진해졌다).
6. **도형 수의 상한 = 900** — ④-UI가 그리는 도형 수는 `LIVE_SHAPE_BUDGET`(900)을 넘지 않는다.
   넘으면 ③이 베이크를 요청하고, 그동안에도 화면은 완전하다(R1·R3). 긴 획 하나(600점)가 만드는
   도형은 733개, 만드는 데 0.1ms다 — 상한은 "느려지면"이 아니라 **구조로** 지킨다.
7. **포인터는 `Border`** — `windows-reactor` 0.100에서 포인터 이벤트는 `Border`에만 있다.
   `Canvas`는 절대 좌표 배치, `Image`는 베이스 전용이다.
8. **입력은 메시지 큐를 거쳐** 처리된다(Reactor의 이벤트 FIFO) — elm의 "이벤트 = 할당"과 맞는다.
9. **필기 자격은 시간이 아니라 장치가 정한다** — WinUI는 장치를 알려주지 않으므로 앱 창을
   서브클래스해 Win32 `WM_POINTER`를 **관찰만** 하고(`DefSubclassProc`으로 그대로 넘긴다),
   `GetPointerType`이 `PT_PEN`인 동안만 잉크가 된다(`tool::CanvasTool::accepts`). 손가락·마우스는
   **무시**하고 취소하지 않는다 — 손바닥이 닿았다고 진행 중인 펜 획을 버리면 필기가 안 된다.
   필압(0~1024)·틸트(-90~90도)는 `GetPointerPenInfo`에서 오고, 장치가 안 보내면(`penMask`)
   속도 기반으로 돌아간다. 프레임에는 **50ms TTL**이 있다 — 펜을 뗀 뒤 온 표본에 펜 자격이
   붙으면 거짓이기 때문이다(`input::FRAME_TTL_MS`).

## 알아둘 한계 (어댑터/런타임 사실)

- **`Image.Source` 교체는 비동기다**: 어댑터는 소스를 먼저 비우고 `SetSourceAsync`로 디코드한
  뒤 완료되면 붙이고 `ImageOpened`를 올린다. 그래서 베이스는 **두 장(더블 버퍼)** 이고, 새
  그림은 창 밖 자리에 넣은 뒤 디코드가 끝난 뒤에만 승격된다(`render::surface` + `Base`).
  소스가 바뀐 그림이 화면에 노출되는 경로가 없어야 깜빡임이 사라진다 — 이 규칙을 깨면 매
  획마다 잉크가 번쩍인다.
- **WinUI `Line`에는 캡/조인 속성이 없다**: `windows-reactor` 0.100이 노출하는 `Line` 속성은
  `Stroke`/`StrokeThickness`/`X1..Y2`뿐이다(`Path`·`Polyline`은 아예 없다). 그래서 둥근 캡은
  **채운 `Ellipse`**, 관절의 둥근 조인은 **구간 사각형을 반지름만큼 늘려** 흉내낸다
  (`shape::extended_span`). 대신 관절 모서리가 `≈0.15r²`만큼 더 채워지는데, 눈에 띄지 않고
  **래스터/PNG/PDF와 라이브가 같은 도형**이 되는 이득이 훨씬 크다(픽셀 차이 0).
- **겹쳐 그리면 반투명 색이 진해진다**: 래스터는 한 획을 한 번의 채움으로 그리므로, 라이브
  레이어도 **한 획 = 한 합성 그룹**(`Canvas.Opacity` = 색의 알파, 자식 도형은 불투명)으로
  그려야 같은 색이 나온다. 이 규칙을 깨면 형광펜의 캡/관절만 진해진다.
- **PDF 내보내기는 폭이 일정한 획을 진짜 베지어 곡선으로 긋는다**(화면 PNG는 라이브 레이어와
  같은 폴리라인 — 오차 0.25px). 벡터 문서에서는 곡선이 더 낫기 때문이고, 확대해도 각지지 않는다.
  폭이 변하는 획은 화면과 **같은 합집합**(늘린 사각형 + 둥근 캡)을 한 번 채운다(관절 얼룩 없음).
- **배경에 없는 페이지를 가리키면 내보내기가 실패한다** — 조용히 흰 종이로 넘어가지 않는다.
  이유를 말하고 멈추는 편이 "왜 빈 페이지가 나왔지"를 만드는 것보다 낫다.
- **필압·틸트는 Win32 `WM_POINTER`에서 온다**: `PointerEventInfo`에는 장치도 압력도 없어서
  앱 창을 서브클래스해 `GetPointerPenInfo`로 읽는다(압력 0~1024 → 0~1, 틸트 -90~90도). 장치가
  압력을 보고하지 않으면(`penMask`) **속도로 굵기를 만든다**(`pressure_from_speed`). 틸트는
  지금 굵기에 쓰지 않는다 — 값은 표본에 실려 있고(상태바가 "필압·틸트 사용 중"으로 확인해 준다)
  도형 결정이 필요해지면 거기서 꺼내 쓴다.
- **디지타이저(펜)만 필기한다**: 손가락·마우스는 화면을 만질 수는 있어도 **획을 만들지 않는다**.
  게이트는 `GetPointerType`이 `PT_PEN`인 프레임이라 펜이 눌린 동안만 잉크가 되고 손바닥은
  무시된다(진행 중인 펜 획은 죽지 않는다). **마우스는 `WM_POINTER`로 오지 않으므로**(프레임이
  없다 = 마우스) 원격 데스크톱·가상 머신처럼 디지타이저가 없는 환경에서는 **필기가 되지
  않는다** — 사고가 아니라 정책이고, 상태바가 이유를 말한다(훅이 안 걸렸으면
  `digitizer::state().reason()`, 펜을 아직 못 봤으면 "아직 펜이 감지되지 않았습니다").
- **창 핸들은 리액터가 공개하지 않는다**: `digitizer`가 `GetActiveWindow` → 이 스레드의 가장 큰
  보이는 창 순으로 **찾아서** 서브클래스를 건다. 창을 못 찾으면 `HookState::NoWindow`가 남고
  상태바가 그 사실을 말한다(조용히 안 그려지는 일이 없다).
- **단축키**: `AcceleratorKey`가 `R`, NumPad, `Add`/`Subtract`, `Enter`만 지원한다 →
  `Ctrl+더하기/빼기`(줌), `Ctrl+Enter`(페이지 추가)만 붙였다. 그래서 같은 동작을
  **툴바 버튼**으로도 제공한다.
- **`<Raw>` 클로저는 슬롯/props를 못 본다**(토큰이 그대로 삽입된다) — 표면 재료는 호스트가
  `ui::stage_frame`으로 넘기고 클로저가 꺼내 쓴다. 표면은 `<Raw>` **하나**뿐이다.
- **콜백 prop은 하나다**(`on_intent`) — 예전에는 버튼마다 18개를 두었고, 그 18개가 화면과
  호스트를 잇는 계약의 전부였다. 지금은 `Intent` 하나가 그 계약이다.
- **자식은 슬롯을 직접 쓰지 않는다** — 공유 값은 호스트가 소유하고 props + 콜백으로 내린다.