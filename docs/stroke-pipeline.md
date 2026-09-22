# 한 획의 파이프라인 — 포인터에서 구운 접두사까지

전제: Windows 11 + `windows-reactor` **0.100.0**. 좌표 단위는 두 가지다 — **pt**(모델: 문서·잉크)와
**픽셀**(그림: WinUI 도형·PNG). 둘을 잇는 값은 하나뿐이다:

```text
scale = Scale::DEFAULT(1.5) × zoom/100        // 1pt = scale 픽셀
픽셀 = pt × scale        pt = 픽셀 / scale
```

## 30초 요약

```text
[UI ①] 포인터 눌림 ──▶ tool.press_with()  → ② 진행 중 획 (O(1))
       이동       ──▶ tool.drag_with()   → shape::live_ink(획) ──▶ WinUI Line+Ellipse
                                                              (즉시, 600점 0.1ms)
       뗌         ──▶ tool.lift() = Edit::AddStroke (커밋)
                        │
                        ├─[UI ③] 그 획은 **꼬리**로 들어간다 (접두사는 여전히 옳다)
                        │        → 화면은 계속 움직인다(멈춤 없음)
                        └─[워커 ④] render::bake(캔버스 스냅샷) = 래스터 + PNG (11.5~28.8ms)
                                   │ HostMessage::Baked(BakeResult)
                                   ▼
                             [UI ③] Base::stage — **화면 밖 자리**에 넣는다(보이지 않는다)
                                   │ ImageOpened → HostMessage::Decoded(index)
                                   ▼
                             [UI ③] Base::promote — 앞뒤 교체(좌표만 바꾼다, 디코드 없음)
                                   → 그 획이 접두사에 들어가 꼬리에서 빠진다
                                   → 라이브 도형과 래스터가 **같은 기하**라 화면은 그대로다
```

핵심은 **두 번의 "따로"**다: ① 무거운 픽셀 작업(워커, R2)과 가벼운 도형(UI, R1)을 다른 스레드에서
만들고, ② 새 그림은 **디코드가 끝난 뒤에만** 화면에 올린다. 그 사이를 꼬리가 메운다(R3).

이 설계에서 **없어진 것**도 계약의 일부다 — 예전에는 ③에 세대 카운터, 150ms 안전망 타이머,
`sleep` 스레드가 있었다. 지금은 **R3 하나**가 그 셋을 대신한다: 접두사가 낡아도 꼬리가 그 획들을
그리므로 화면이 완전하고, 그래서 "언제 승격됐는가"를 셀 이유가 없다(장부는 `last_request` 하나).

## 참여자 (누가 무엇을 소유하는가)

| 주체 | 소유 | 코드 |
|---|---|---|
| 셸(호스트) | 문서·PDF·도구·줌·**④-워커 예약** | `src/app.rs` (`Shell`) |
| elm 화면 | 툴바/사이드바/상태바 + `<Raw>` **하나** | `src/ui.rs` |
| WinUI 트리 | 컨트롤 트리(베이스 `Image` 2장 + 꼬리 그룹) | `src/render.rs` |
| ③ 캔버스 | 문서 + **구운 접두사**(`Base`) + **꼬리**(`Rc<[LiveInk]>`) | `src/canvas.rs` |
| 모델 | 페이지·획·Undo/Redo | `src/doc.rs`, `src/ink.rs` |
| 기하/래스터 | 도형 결정(`ink_shape`) + 픽셀화 | `src/shape.rs` |
| 입력 | 표면 DIP → pt, 위상, **장치·필압·틸트** | `src/input.rs`, `src/digitizer.rs` |
| 도구 | press/drag/lift/cancel, **펜만 필기**, 압력(필압, 없으면 속도) | `src/tool.rs` |

**스레드 2개만 있다**: UI 스레드(= Reactor 메시지 루프 + elm 프레임)와 백그라운드 워커
(`context.spawn_background`). 워커로 가는 것은 **`Send`인 스냅샷**뿐이고(획 목록 복사, PNG 바이트),
PDF 문서는 넘어가지 않는다 — 워커가 PDF가 필요하면 바이트에서 다시 파싱한다.

## 프레임 루프 (모든 메시지가 지나가는 길)

`Shell::update()`는 메시지 하나를 처리한 뒤 **항상** `refresh()`를 부른다(`app.rs`).

```rust
fn update(&mut self, message: HostMessage, context: &ComponentContext<Self>) {
    match message { /* Pointer | Intent | Baked | Decoded | Opened | Exported */ }
    self.refresh(context);        // ③을 최신으로 + 필요하면 ④-워커에 **한 건**
}
```

`Shell::view()`가 매 프레임 하는 일(순서가 계약이다):

| # | 하는 일 | 왜 |
|---|---|---|
| ⓪ | `digitizer::install()` — 앱 창을 찾아 `WM_POINTER` 서브클래스를 건다(`view()`/`update()`에서 매번 시도) | 창이 아직 없으면 다음 기회에 다시 시도한다(여러 번 불러도 한 번만 걸린다) |
| ① | 포인터 싱크 등록 — `Rc<dyn Fn(phase,x,y)>`가 **이벤트 순간의 펜 프레임**을 붙여 `HostMessage::Pointer`를 큐에 넣는다 | 포인터 이벤트는 `Border`에만 있고, 표면 빌더가 이 싱크를 **캡처**한다(예제 08) |
| ② | 디코드 싱크 등록 — `ImageOpened` → `HostMessage::Decoded(index)` | **승격의 유일한 신호** |
| ③ | 가속기 등록 → 같은 큐 | `Ctrl +/-`, `Ctrl+Enter`만 지원(어댑터 한계) |
| ④ | `ui::stage_frame(self.canvas.frame())` | `<Raw>` 클로저는 props/슬롯을 못 본다 — 발행 직전에 재료를 넣는다 |
| ⑤ | `ScreenProps { view, on_intent }` → `ElmView<Screen>` | 호스트 → elm은 props, elm → 호스트는 **콜백 하나** |

그리고 elm이 `<Raw>`를 만나면(`ui.rs`):

```rust
<Raw>|out: &mut SurfaceSlot| {
    if let (Some(frame), Some(builder)) = (pending_frame(), surface_builder()) {
        *out = builder(&frame);          // → render::surface(트리)
    }
}</Raw>
```

`Frame`은 **스냅샷**이다: `base`(`Rc` 2장 + 페이지/배율/획 수) / `tail`(`Rc<[LiveInk]>`) /
`drawing` / `size`·`scale`·`baked`·`strokes`. 꼬리 전체가 `Rc`라 **프레임마다 복사가 없고**
(`frame()` 실측 0.0ms), `pending_frame()`은 소비되지 않는다 — 한 발행이 여러 번 그려도 같은 것을
본다(비우는 것은 호스트가 `clear_frame()`으로 명시적으로 한다).

## 단계별 파이프라인

### T0 — 포인터 눌림 (`Pressed`)

```text
WM_POINTERDOWN → GetPointerType → GetPointerPenInfo   src/digitizer.rs (창 서브클래스, 관찰만)
  └ LATEST(스레드 로컬) = PointerFrame { device, pressure, tilt }
Border.on_pointer_pressed(info)                     src/render.rs
  └ InputSink(Pressed, info.x, info.y) → LocalSender → HostMessage::Pointer   ← 프레임을 붙인다
      └ Shell::pointer()                            src/app.rs
           sample = input::sample_with(Pressed, x, y, canvas.scale(), frame)   // ① DIP → pt + 장치
           CanvasTool::accepts(sample.device())?        // **펜만 통과** — 아니면 상태바에 이유
           tool.press_with(canvas.doc_mut(), sample.at, sample.now, sample.pressure())  // ② 진행 중 획
```

- 포인터 이벤트는 **동기 처리가 아니다**: Reactor의 이벤트 FIFO를 거쳐 `update()`에서 돈다.
  그래서 `update()` 안에서 오래 걸리는 일(래스터)을 하면 **그 프레임이 통째로 멈춘다**.
- ①은 곱셈 하나다: `Pt = (x / scale, y / scale)`. 페이지 크기와 무관하다(R1).
- ②는 O(1)이다 — 획 하나(점 하나)를 만들 뿐이다.
- **필기 자격은 장치가 정한다**: `WM_POINTER`를 읽어 `PT_PEN`일 때만 잉크가 되고, 손가락·마우스는
  무시한다(취소하지 않는다 — 손바닥이 닿았다고 펜 획을 버리면 필기가 안 된다). 끝(`Released`/
  `Canceled`)은 장치와 무관하게 전달한다: 커밋할 제스처는 펜이 시작한 것뿐이다.
- 압력은 **하드웨어가 보고하면 그것**(`GetPointerPenInfo`, 0~1024 → 0~1), 아니면 **속도로
  만든다**: `pressure_from_speed(pt/ms)` → 빠르면 가늘고 느리면 굵다. 폭은
  `Style::width_at(pressure) = width_pt × (0.45 + 0.55 × pressure)`.
- 프레임은 **이벤트 순간**의 것만 쓴다(`FRAME_TTL_MS = 50`): 펜을 뗀 뒤 온 표본에 펜 자격이
  붙으면 장치도 압력도 거짓이 된다.
- **접두사는 손대지 않는다** — 진행 중 획은 꼬리가 그린다.

### T1 — 포인터 이동 (`Moved`) = 필기 중 매 프레임

```text
tool.drag_with(canvas.doc_mut(), sample.at, sample.now, sample.pressure())   src/tool.rs → src/doc.rs
  · 최소 간격(0.6pt)보다 가깝고 압력 차도 작으면 **버린다**(모델을 가볍게 유지)
  · 필압이 오면 그대로 쓰고, 없으면 속도 추정이라 "가깝지만 압력이 다르다"는 표본이 남는다
    → 폭이 변하는 획이 흔하다

canvas.refresh(tool.drawing())                        src/canvas.rs
  ├ 꼬리: tail_dirty/꼬리 시작·길이가 어긋났을 때만 **다시 계산**(보통은 그대로)
  └ 진행 중 획만 shape::live_ink(획, scale)  ← ink_shape 와 **같은 도형**
view() → ui::stage_frame → <Raw> → render::surface(frame, sink, decoded, accelerators)
  └ Canvas.keyed_children([base-0, base-1, tail-0 …, drawing])
      └ "drawing" = Canvas(opacity = α/255)                  ← 한 획 = 한 합성 그룹
          ├ Line    "line-i"  (구간, 곡선을 편 직선 조각 — 양 끝을 반지름만큼 늘림)
          └ Ellipse "cap-i"   (둥근 캡 — Line에는 캡 속성이 없다)
```

- 비용은 **진행 중 획 하나**에만 비례한다: `live_ink` 600점 **0.1ms**, `refresh`(진행 중 획만
  갱신) **0.1ms**. 나머지 프레임 비용(`frame()`/`Frame` 복사)은 **0.0ms**(`Rc` 복사).
- Reactor는 **키로 diff**한다(`drawing`, `line-i`, `cap-i`, …) — 600점이어도 매 프레임 바뀌는
  것은 마지막 선분 몇 개와 새로 붙은 캡뿐이다.
- 라이브 도형의 색은 **불투명**하고 투명도는 그룹이 맡는다 — 자식이 겹쳐도(캡/관절) 알파가
  두 번 곱해지지 않는다(래스터의 "한 번의 채움"과 같은 결과 → 형광펜이 얼룩지지 않는다).

### T2 — 포인터 뗌 (`Released`) = 확정 = **여기가 경계다**

```text
Shell::pointer(Released)
  edit = tool.lift(canvas.doc_mut())      // ② → Edit::AddStroke / Edit::RemoveStrokes
  match edit {
      Some(Edit::AddStroke { .. })     => { /* ③은 놔둔다 — 이 획은 꼬리로 간다 */ }
      Some(Edit::RemoveStrokes { .. }) => canvas.invalidate(),   // 지우개 — 픽셀에서 뺄 수 없다
      _                                => {}
  }
  refresh(context)                        // 같은 update() 안에서 바로
```

- **추가는 접두사를 낡게 하지 않는다**(이 설계의 핵심 결정): 접두사는 `0..baked_count`의 획만
  담고 있고 그 획들은 그대로다. 새 획은 꼬리(`tail`)에 붙고, ④는 키가 바뀐 그룹 하나만 다시 그린다.
  → **획 하나마다 30ms짜리 베이크가 붙지 않는다.**
- `RemoveStrokes`/Undo/Redo/페이지 이동/줌은 `Canvas::invalidate()` — 화면의 접두사가 **옛 상태**라
  새 그림이 필요하다. 그래도 **화면은 지우지 않는다**(새 그림이 승격될 때까지 옛 접두사 + 꼬리).
  지운 잉크가 사라지는 시점은 새 접두사가 도착한 뒤다(≈15ms) — 화면이 **비는** 것과는 다르다.
- 캡처 유실/시스템 취소는 `tool.cancel()` — 진행 중 획과 **지우개 결과를 되돌리고 편집을 남기지
  않는다**(히스토리에도 안 남는다). 두 경로(`capture_lost`, `canceled`)가 같은 위상으로 온다.

### T3 — 워커: 래스터 + PNG 인코딩 (UI 스레드 밖)

```text
Shell::refresh(context): canvas.needs_bake(baking.is_some())
  └ canvas.bake_request()                       src/canvas.rs
       · stale = false            (이 요청이 접두사를 새로 만든다)
       · last_request = Some(id)  (**이 순간부터 낡은 응답은 안 온다**)
       · BakeRequest { id, page, size, scale, count, strokes: 획 목록 **복사**(Send) }
  └ if request.is_empty() → canvas.accept(request.empty_result())   // 잉크 없음: 워커를 안 부른다
     else spawn_background(move |_cancel| HostMessage::Baked(render::bake(&request)))
```

워커 안에서(`src/render.rs` → `src/shape.rs`):

```text
render::bake(request)                              // size = 페이지 pt, scale = 1.5×줌
  shape::has_ink: 지우개가 아닌 비어있지 않은 획이 하나라도 있는가   ← 픽셀 스캔 없이 즉시
  shape::render_ink: RenderContext(893×1263) 에 획마다 fill_path(합집합) **한 번**
  shape::to_png: vello Pixmap → PNG 바이트 (Arc<[u8]>)
```

**한 획 = 한 번의 `fill_path`** 가 여기서 지켜진다(관절·캡이 겹쳐도 알파가 두 번 곱해지지 않는다).
`needs_bake(inflight)`가 **한 번에 하나만** 허용한다(R2: latest-wins).

### T4 — 도착 → **화면 밖 자리**에 스테이징

```text
HostMessage::Baked(result) → Shell::adopt_bake() → canvas.accept(result)
  · last_request != Some(result.id)       → None : 낡은 응답(그 사이 더 새 요청/편집이 있었다)
  · result.page/scale != 지금 페이지/배율 → None : 쓸 수 없다 → stale = true (다시 굽는다)
  · 그 외                                  → Some(Base::stage(..))
      Base::stage(png, page, scale, count) → **창 밖 자리**에만 쓴다
        · 같은 그림(Arc 동일성)이면 true → 디코드를 기다릴 필요가 없다(즉시 promote)
        · 아니면 false → ImageOpened 신호를 기다린다
```

- **보이는 자리는 손대지 않는다.** 화면 밖 `Image`는 `(-10000, -10000)`에 있다(`Image`에는
  visibility/opacity가 없고, 크기를 0으로 줄이면 WinUI가 디코드를 게으르게 할 수 있다).
- `stage`는 **페이지·배율·획 수**를 함께 기억한다 — 그 그림을 승격해도 되는지 판단하는 유일한
  근거다(`Base::is_valid_for(page, scale)`).

### T5 — 디코드 완료 = 승격 신호

WinUI가 화면 밖 레이어의 `SetSourceAsync`를 끝내면 어댑터가 `ImageOpened`를 올린다:

```text
Image.on_opened → ImageSink(index) → HostMessage::Decoded(index) → Shell::update
  canvas.promote()      // 대기 중 그림이 **지금 페이지·배율**일 때만 좌표를 맞바꾼다
```

`Image.Source` 교체는 어댑터 안에서 **소스를 먼저 비우고** 비동기로 디코드한다
(`windows-reactor` 0.100) — 그래서 한 장만 쓰면 매 획마다 잉크가 통째로 사라진다. 두 장 + 승격
신호가 그 경로를 **구조적으로** 막는다.

### T6 — 승격 직후: 꼬리가 저절로 줄어든다

```text
update() 끝 → canvas.refresh(...)
  tail = committed[base.count() ..]   // base.count()가 방금 늘었다 → 방금 승격된 획이 빠진다
  → 라이브 도형에서 그 획이 사라진다. 화면은 **아무것도 안 바뀐 것처럼 보인다**:
    라이브 도형(PNG가 도착하기 전 그린 것)과 래스터 잉크가 **같은 기하**이기 때문이다.
```

이 "아무것도 안 바뀐 것처럼"이 코드로 보장되는 부분:

| 무엇 | 어떻게 같게 만드는가 |
|---|---|
| 곡선 모양 | 래스터도 라이브와 **같은 폴리라인**을 채운다 (`flatten_spans`, 오차 0.25px) |
| 구간·관절 | 구간 사각형을 **반지름만큼 늘려** 둥근 조인을 덮는다 (`extended_span`) |
| 획 끝 | 양 끝에 **같은 반지름의 둥근 캡**(래스터는 원, 라이브는 `Ellipse`) |
| 점 하나(탭) | 둘 다 **채운 원 하나**(선분 없음) |
| 반투명 색 | 래스터는 한 번의 채움, 라이브는 **한 획 = 한 합성 그룹**(`Canvas.Opacity`) |
| 검증 | `tests/geometry.rs`가 **바이트로** 비교 — 곡선/가변폭/점/형광펜 **차이 0** |

### T7 — 다음 획 / 예산 초과

다음 표본은 다시 T1부터다. 꼬리가 커져서(예: 긴 획 하나 → 도형 733개) ④-UI가 만드는 도형 수가
`LIVE_SHAPE_BUDGET`(900)을 넘으면 ③이 `needs_bake`로 답한다 — 그때는 **베이크 한 번**으로 접두사가
자라 꼬리가 통째로 비워진다. 렌더 중에 또 그으면 **한 번만** 돈다(진행 중이면 요청을 만들지 않고,
끝나면 최신 스냅샷으로 한 번 더).

## 베이스 상태 기계 (`canvas::Base` — 자리 두 개)

```text
slots = [ Slot { png: None, page: 0, scale, count: 0 },
          Slot { png: None, page: 0, scale, count: 0 } ]     visible = 0

        visible ── 화면에 보이는 자리(0,0), 페이지 크기
        hidden  ── 창 밖(-10000,-10000) 자리 — **새 그림은 언제나 여기로**
                     │
   stage(png, page, scale, count)      ← T4    (같은 그림이면 true = 기다릴 것 없음)
                     ▼
        hidden = { png, page, scale, count }
                     │
        promote(page)                  ← T5 (ImageOpened) — 자리의 page/scale이 지금과
                     ▼                                    다르면 **거부**한다(좌표만 맞바꾼다)
        visible ↔ hidden, 둘 중 새 그림이 화면으로

   drop_staged()                       ← 페이지 이동: 대기 중 그림은 다른 페이지의 것이다
   is_valid_for(page, scale)           ← 지금 화면의 접두사가 이 페이지·배율의 것인가
```

- `count()` = **보이는 그림이 반영한 획 수** — 꼬리 계산의 유일한 기준이다.
- `visible_png()`가 `None`이면 접두사가 없다: 화면은 꼬리만으로 그려진다(빈 페이지/배율 변경 직후).
- **타이머가 없다.** 디코드 신호가 늦게 오거나 오지 않아도 화면은 옛 접두사 + 꼬리로 완전하다(R3).
  신호가 오면 그때 좌표만 맞바꾼다 — 승격이 늦어도 잃는 것은 "꼬리가 조금 더 그리는 일"뿐이다.

## 꼬리(tail) 규칙 한 줄

```text
라이브로 그릴 것 = committed[base.count() ..]  +  진행 중 획
```

`Canvas::refresh`는 **꼬리의 시작(`base.count()`)이나 길이(확정 획 수)가 달라졌을 때만** 다시
계산한다(`tail_dirty`는 지우개·되돌리기처럼 획 **집합**이 바뀐 경우). 그래서 평소 프레임에서는
진행 중 획 하나만 새로 만든다 — 이 규칙이 R1의 실체다.

| 상황 | `base.count()` | 화면 |
|---|---|---|
| 필기 중 | `N` | 접두사는 N획, 라이브가 진행 중 획 |
| 뗀 직후(워커 도는 중) | `N` | 접두사는 N획, **라이브가 방금 뗀 획**(꼬리) |
| 승격 완료 | `N+1` | 접두사가 N+1획, 라이브에서 그 획이 빠짐(모양은 그대로) |
| 지우개·되돌리기 | `N`(옛 값) | 옛 접두사 + 남은 획들이 꼬리에 — 새 접두사가 오면 정리된다 |
| 페이지 이동 | 새 페이지의 값 | 이전 페이지 그림은 **버려진다**(`drop_staged`) |

## 예외 경로 (전부 "화면이 비지 않는다"로 수렴한다)

| 사건 | 처리 | 근거 |
|---|---|---|
| 포인터 캡처 유실 / 시스템 취소 | `tool.cancel()` → 진행 중 획·지우개 결과 복구, **편집 기록 없음** | `tool.rs`, `app.rs` |
| 지우개 드래그 | `Edit::RemoveStrokes` → `Canvas::invalidate()` (픽셀에서 잉크를 뺄 수 없다) | `canvas.rs` |
| 되돌리기/다시하기/비우기 | `Canvas::edit()` — 모델 편집과 무효화가 **한 곳**이라 잊을 수 없다 | `canvas.rs` |
| 렌더 중 또 그림 | `needs_bake(inflight)` + `last_request` → 낡은 응답은 `accept`가 `None`으로 버린다 | `canvas.rs` |
| 디코드 신호가 안 옴 | **기다리지 않는다** — 옛 접두사 + 꼬리로 완전(타이머·안전망 없음) | `canvas.rs` |
| 페이지 이동 / 줌 | `drop_staged()` + `invalidate()` — 대기 중 그림은 다른 페이지·배율의 것이다 | `canvas.rs` |
| 빈 페이지 | `has_ink == false` → 워커를 부르지 않고 `empty_result()`(그림 없음 = 빈 접두사) | `canvas.rs` |
| 자리를 채우는 오류 | `Stage::Failed(이유)` + **같은 자리에** "다시 시도" 버튼 | `ui.rs` |
| 파일 열기/내보내기 | 워커에서 처리, 결과만 메시지로(`PdfDocument`는 넘기지 않는다 — `Send` 제약) | `app.rs`, `files.rs`, `export.rs` |

## 비용 (release, A4 595×842pt @ scale 1.5 = 893×1263px, 예제 `bake_cost` 실측)

| 단계 | 스레드 | 실측 |
|---|---|---|
| 포인터 1점 (`press`/`drag`/`lift` + `refresh`) | UI | **0.0~0.1ms** |
| `shape::live_ink` 600점 | UI | **0.1ms** |
| `Canvas::frame` / `Frame` 복사 | UI | **0.0ms** |
| `render::bake` A4 — 1획 / 20획 / 300획 | 워커 | 11.5 / 14.0 / **28.8ms** |
| ├ `shape::render_ink` (20획) | 워커 | 2.1ms |
| └ `shape::to_png` (인코딩이 대부분) | 워커 | **14.3ms** |
| 같은 코드 **dev 빌드**(opt-level 0) | 워커 | 20배 ← 그래서 워커다 |

`cargo run --release -p light-note-gui --example bake_cost`로 언제든 다시 잰다.

## WinUI 트리 (키 포함)

```text
Grid                        key_accelerators(Ctrl+±, Ctrl+Enter)
└ Border                    종이 배경 + **포인터 이벤트**(Canvas에는 없다)
  └ Canvas                  width/height = 페이지 픽셀(=DIP), 절대 좌표
    ├ Image  "base-0"       베이스 자리 A (EncodedImage, Stretch::None)
    ├ Image  "base-1"       베이스 자리 B — 보이는 쪽만 (0,0), 나머지는 (-10000,-10000)
    ├ Canvas "tail-i"       꼬리 획 i = 한 합성 그룹, Opacity = 색의 알파/255
    │ ├ Line    "line-j"    구간 사각형(양 끝을 반지름만큼 늘림), StrokeThickness = 폭
    │ └ Ellipse "cap-j"     둥근 캡(지름 = 폭), Canvas.Left/Top = 중심 - 반지름
    └ Canvas "drawing"      진행 중 획 (같은 모양)
```

`KeyedView`의 키가 곧 diff 단위다 — 도형이 늘어도 앞쪽은 그대로 두고 끝만 바뀐다.

## 불변식 ↔ 테스트

| 불변식 | 테스트 |
|---|---|
| 꼬리는 **구운 접두사 뒤에서 시작**한다 | `pipeline::stage3_the_tail_starts_where_the_base_ends` |
| 추가는 접두사를 낡게 하지 않는다(베이크를 안 부른다) | `pipeline::stage2_committing_a_stroke_does_not_invalidate_the_base` |
| 낡은 응답은 버리고 다시 굽는다 | `pipeline::stage3_a_late_answer_is_dropped` |
| 지우개는 재베이크를 강제한다 | `pipeline::stage3_erasing_invalidates_the_base` |
| 다른 페이지의 그림은 승격되지 않는다 | `pipeline::stage3_page_move_drops_the_staged_picture` |
| 예산이 UI 도형 수의 상한(R1) | `pipeline::stage3_budget_caps_the_ui_shape_count` |
| 드래그 하나 = 편집 하나 / 취소는 흔적 없음 | `pipeline::stage2_a_drag_becomes_exactly_one_edit`, `stage2_cancel_leaves_nothing_behind` |
| 표면 DIP → pt 규약 | `pipeline::stage1_normalizes_surface_dip_to_points` |
| 라이브 도형 = 래스터 잉크(바이트 차이 0) | `geometry::*_matches_between_live_and_raster` |
| 관절을 덮는 확장 규칙 / 이중 합성 없음 | `geometry::the_joint_is_covered_by_the_extended_span`, `a_translucent_joint_does_not_double_blend` |
| 화면 → 의도 매핑과 상태 분기 | `ui_plan::*` |
| 표면은 `<Raw>` **하나**로 붙고 재료가 그대로 도착 | `ui_plan::the_surface_reaches_the_registered_builder_through_one_raw_slot` |
| 인코딩 계약(BOM·UTF-8·CRLF) | `encoding::*` |

## 더 읽을 곳

- `README.md` — 실행/전제조건, 최적화 목록, 인코딩 규칙, 계측 표
- `crates/light-note-gui/src/canvas.rs` — ③(접두사·꼬리·베이크 예약·승격)
- `crates/light-note-gui/src/shape.rs` — 도형 결정(`ink_shape`)·폴리라인 펴기·합집합 채움
- `crates/light-note-gui/src/render.rs` — ④(WinUI 트리·베이크·가속기)
- `crates/light-note-gui/src/app.rs` — 셸(메시지·워커·승격)