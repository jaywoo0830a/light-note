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
| `core::surface` | WinUI 표면이 무엇을 그릴지 (정적 PNG + 라이브 선분) |
| `core::ui` | elm-magic 화면 + 어댑터 계획(plan) 계약 |

## 테스트 (리눅스에서 전부 돈다)

```bash
cargo test -p light-note-core
```

51개 테스트가 **UI 계약까지** 검증한다:

- `ink_model` — 샘플 필터/필압/경계/지우개 히트 테스트
- `document` — Undo/Redo(지우개 드래그 = 편집 하나), 페이지 관리, 취소
- `raster` — 잉크가 **어디에** 올라갔는지 픽셀로 확인, 결정성, PNG 왕복
- `export` — 내보낸 PDF를 **hayro로 다시 읽어** 배경/잉크 위치까지 검증
- `ui_plan` — 버튼 → 의도 매핑, 상태 분기(로딩/빈/준비/실패), `<Raw>` 표면 전달

## 실행 (윈도우 11)

```powershell
cargo run -p light-note-win --release
```

## Windows 11 최적화 포인트

1. **확정 레이어 = PNG 한 장** — 드래그가 *끝날 때만* 다시 만든다(`InkSurface` 계약).
2. **라이브 레이어 = WinUI `Line` 몇 개** — GPU 합성이라 포인터에 즉시 반응한다.
3. **포인터는 `Border`** — `windows-reactor` 0.100에서 포인터 이벤트는 `Border`에만 있다.
   `Canvas`는 절대 좌표 배치, `Image`는 정적 레이어 전용이다.
4. **입력은 메시지 큐를 거쳐** 처리된다(Reactor의 이벤트 FIFO) — elm의 "이벤트 = 할당"과 맞는다.

## 알아둘 한계 (어댑터/런타임 사실)

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

