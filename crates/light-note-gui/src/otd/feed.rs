//! 펜 표본 흐름 → **제스처** — ①의 태블릿 절반.
//!
//! 공유 메모리에서 오는 것은 "펜이 어디에 있고 얼마나 눌렸는가"뿐이다. ②(도구)가 필요로
//! 하는 것은 **위상이 붙은 표본**이다: 시작(`Pressed`) · 진행(`Moved`) · 끝(`Released`).
//! 이 파일이 그 사이의 유일한 변환이다 — 그리고 [`crate::input::Sample`]을 **그대로** 만들어
//! 내보내므로, ② 이후는 입력이 WinUI에서 왔는지 태블릿에서 왔는지 모른다.
//!
//! ## 위상은 **접촉**에서 나온다 (플러그인의 `FlagTip` = `pressure > 0`)
//!
//! | 표본 | 위상 |
//! |---|---|
//! | 닿았다(직전에는 안 닿았음) | `Pressed` |
//! | 닿았다(직전에도 닿았음) | `Moved` |
//! | 안 닿았다(직전에는 닿았음) | `Released` |
//! | 범위 이탈(그리는 중) | `Released` — 진행 중 획을 **커밋**한다 |
//! | 호버(안 닿음이 계속) | 없음 — 잉크가 아니다 |
//!
//! 범위 이탈이 `Canceled`가 아닌 이유: 획을 그리다 태블릿 밖으로 나가는 일은 흔하고(특히
//! 가장자리), 그때 그린 것을 **버리면** 사용자는 "획이 사라졌다"고 느낀다. 취소는 시스템이
//! 입력을 끊었을 때만이다 — [`Feed::finish`]가 그 자리다(플러그인이 사라졌거나 앱이 끝날 때).
//!
//! ## 표본 시각은 앱이 잡는다
//! 공유 메모리의 `t`(QPC)는 쓰지 않는다: ②의 속도 추정은 `Instant` 기준이라 두 시계를 섞으면
//! 음수 구간이 생긴다. 400 Hz 표본을 1 ms 폴링으로 읽으면 시각 오차는 최대 1 ms인데, 필압은
//! **장치가 보고하므로** 속도 추정이 쓰이지 않는다(오차가 잉크에 닿지 않는다).

use std::time::Instant;

use crate::geom::{Pt, Size};
use crate::input::{Device, Phase, PointerFrame, Sample};
use crate::otd::map::Mapper;
use crate::otd::shm::PenSample;
use crate::otd::wire::TabletSpec;

/// 표본 흐름 → 표본. **상태는 셋뿐이다**: 변환기, 접촉 중인가, 마지막 점.
pub struct Feed {
    /// 태블릿 → 페이지. 페이지 크기가 바뀌면 다시 만든다([`Feed::retarget`]).
    mapper: Mapper,
    /// 직전 표본이 접촉이었는가 — `Pressed`/`Moved`/`Released`를 가르는 유일한 근거다.
    contact: bool,
    /// 마지막으로 **페이지 위에 있던** 점. 범위 이탈에서 획을 끝낼 자리다.
    last: Option<Pt>,
}

impl Feed {
    /// 태블릿 범위와 페이지 크기로 시작한다.
    pub fn new(tablet: &TabletSpec, page: Size) -> Self {
        Self {
            mapper: Mapper::fit(tablet, page),
            contact: false,
            last: None,
        }
    }

    /// 태블릿 범위나 페이지 크기가 바뀌었다 — **획을 끊지 않는다**(그리는 중이면 좌표만 바뀐다).
    ///
    /// 줌·페이지 이동에서 WinUI 입력도 같은 일을 겪었다: 좌표는 화면 기준이 아니라 문서
    /// 기준이므로, 바뀐 것은 배율뿐이다.
    pub fn retarget(&mut self, tablet: &TabletSpec, page: Size) {
        self.mapper = Mapper::fit(tablet, page);
    }

    /// 지금 그리는 중인가 — 진단용.
    pub fn is_drawing(&self) -> bool {
        self.contact
    }

    /// 표본 하나를 흘려보낸다. **잉크가 아닌 표본(호버)은 아무것도 내보내지 않는다.**
    pub fn push(&mut self, sample: &PenSample, now: Instant, out: &mut Vec<Sample>) {
        if sample.ends_the_stroke() {
            // 범위 이탈 — 이 표본의 좌표는 무효다(플러그인이 0을 쓴다). 그리는 중이었으면
            // **마지막 점에서** 커밋한다(그린 것을 버리지 않는다).
            self.contact = false;
            if let Some(at) = self.last.take() {
                out.push(Sample {
                    phase: Phase::Released,
                    at,
                    now,
                    frame: Some(self.frame(0.0, None, false, now)),
                });
            }
            return;
        }

        let at = self.mapper.to_page(sample.x, sample.y);
        let pressure = self.mapper.pressure(sample.pressure);
        let frame = self.frame(pressure, sample.tilt, sample.eraser, now);

        if sample.contact {
            let phase = if self.contact {
                Phase::Moved
            } else {
                Phase::Pressed
            };
            self.contact = true;
            self.last = Some(at);
            out.push(Sample {
                phase,
                at,
                now,
                frame: Some(frame),
            });
        } else if self.contact {
            // 손을 뗐다 — 마지막 점은 그대로 두고 **끝**만 알린다(②가 커밋한다).
            self.contact = false;
            out.push(Sample {
                phase: Phase::Released,
                at,
                now,
                frame: Some(frame),
            });
        }
        // 호버(안 닿음이 계속)는 잉크가 아니다 — 표본을 만들지 않는다.
    }

    /// 표본 흐름이 **끊겼다** — 그리는 중이었으면 취소한다.
    ///
    /// 플러그인이 사라지거나(매핑이 닫힘) 앱이 끝날 때 온다. 여기서 커밋하면 반쪽 획이
    /// 문서에 남으므로 취소가 맞다 — ②의 [`crate::tool::CanvasTool::cancel`]과 같은 뜻이다.
    pub fn finish(&mut self, now: Instant, out: &mut Vec<Sample>) {
        self.last = None;
        if !self.contact {
            return;
        }
        self.contact = false;
        out.push(Sample {
            phase: Phase::Canceled,
            at: Pt::new(0.0, 0.0),
            now,
            frame: None,
        });
    }

    /// 하드웨어 프레임 하나 — 장치는 **언제나 펜**이다(태블릿이 보낸 표본이므로).
    ///
    /// `has_eraser`는 `true`다: OTD가 지우개 비트를 보고한다는 사실 자체가 장치에 지우개
    /// 끝이 있다는 뜻이다(뒤집힘 여부는 `inverted`가 따로 말한다).
    fn frame(
        &self,
        pressure: f32,
        tilt: Option<(f32, f32)>,
        eraser: bool,
        now: Instant,
    ) -> PointerFrame {
        PointerFrame::new(Device::Pen, Some(pressure), tilt, now)
            // 회전은 0.6.7에 리포트가 없다 — 계약이 0을 쓰고, 여기서도 `None`이다.
            .with_pen_pose(eraser, true, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otd::shm::{FLAG_ERASER, FLAG_OUT_OF_RANGE, FLAG_TIP};

    /// 실측한 태블릿 — 이 PC의 XP-Pen Deco 01 V3 (Variant 2).
    fn deco() -> TabletSpec {
        TabletSpec {
            name: "XP-Pen Deco 01 V3 (Variant 2)".to_string(),
            max_x: 51196.0,
            max_y: 31826.0,
            max_pressure: 16383.0,
        }
    }

    fn sample(x: f32, y: f32, pressure: f32, flags: u32) -> PenSample {
        PenSample {
            seq: 1,
            x,
            y,
            pressure,
            tilt: None,
            eraser: flags & FLAG_ERASER != 0,
            buttons: 0,
            contact: flags & FLAG_TIP != 0,
            out_of_range: flags & FLAG_OUT_OF_RANGE != 0,
        }
    }

    fn feed() -> Feed {
        Feed::new(&deco(), Size::A4)
    }

    #[test]
    fn contact_begins_and_ends_a_stroke() {
        // 호버 → 접촉 → 접촉 → 뗌: 이것이 획 하나의 전부다.
        let mut feed = feed();
        let now = Instant::now();
        let mut out = Vec::new();
        feed.push(&sample(1000.0, 1000.0, 0.0, 0), now, &mut out);
        assert!(out.is_empty(), "호버는 표본이 아니다");

        feed.push(&sample(1000.0, 1000.0, 4000.0, FLAG_TIP), now, &mut out);
        feed.push(&sample(2000.0, 1000.0, 5000.0, FLAG_TIP), now, &mut out);
        feed.push(&sample(2000.0, 1000.0, 0.0, 0), now, &mut out);
        assert_eq!(
            out.iter().map(|sample| sample.phase).collect::<Vec<_>>(),
            vec![Phase::Pressed, Phase::Moved, Phase::Released]
        );
        assert!(out.iter().all(|sample| sample.is_ink()), "전부 펜이다");
        assert!(!feed.is_drawing(), "뗐다");
    }

    #[test]
    fn pressure_and_tilt_come_from_the_device() {
        let mut feed = feed();
        let now = Instant::now();
        let mut out = Vec::new();
        let mut pressed = sample(1000.0, 1000.0, 16383.0, FLAG_TIP);
        pressed.tilt = Some((12.5, -30.0));
        feed.push(&pressed, now, &mut out);
        assert_eq!(out[0].pressure(), Some(1.0), "장치 최대값이 기준이다");
        assert_eq!(out[0].tilt(), Some((12.5, -30.0)));

        // 절반 필압.
        let mut out2 = Vec::new();
        feed.push(&sample(1100.0, 1000.0, 8191.5, FLAG_TIP), now, &mut out2);
        let half = out2[0].pressure().expect("필압");
        assert!((half - 0.5).abs() < 0.001, "{half}");
    }

    #[test]
    fn the_eraser_bit_flips_the_pen() {
        // 뒤집힌 펜은 **그 제스처만** 지운다 — 도구 선택은 ②가 지킨다.
        let mut feed = feed();
        let now = Instant::now();
        let mut out = Vec::new();
        feed.push(
            &sample(1000.0, 1000.0, 4000.0, FLAG_TIP | FLAG_ERASER),
            now,
            &mut out,
        );
        assert!(out[0].inverted(), "지우개 끝이다");
        assert_eq!(out[0].device(), Device::Pen);
    }

    #[test]
    fn out_of_range_commits_the_stroke() {
        // 태블릿 밖으로 나갔다 — 그린 것을 **버리지 않는다**(마지막 점에서 커밋한다).
        let mut feed = feed();
        let now = Instant::now();
        let mut out = Vec::new();
        feed.push(&sample(1000.0, 1000.0, 4000.0, FLAG_TIP), now, &mut out);
        let started = out[0].at;
        out.clear();
        feed.push(&sample(1200.0, 1200.0, 4000.0, FLAG_TIP), now, &mut out);
        let last = out[0].at;
        out.clear();

        feed.push(&sample(0.0, 0.0, 0.0, FLAG_OUT_OF_RANGE), now, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].phase, Phase::Released);
        assert_eq!(out[0].at, last, "마지막 점에서 끝난다: {started:?}");
        assert_eq!(out[0].pressure(), Some(0.0), "떨어진 펜은 필압이 없다");

        // 범위 밖에서 또 오면 아무것도 내보내지 않는다(이미 끝났다).
        out.clear();
        feed.push(&sample(0.0, 0.0, 0.0, FLAG_OUT_OF_RANGE), now, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn finish_cancels_an_unfinished_stroke() {
        // 플러그인이 사라졌다 — 반쪽 획을 문서에 남기지 않는다.
        let mut feed = feed();
        let now = Instant::now();
        let mut out = Vec::new();
        feed.push(&sample(1000.0, 1000.0, 4000.0, FLAG_TIP), now, &mut out);
        out.clear();
        feed.finish(now, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].phase, Phase::Canceled);
        assert!(!feed.is_drawing());

        // 이미 뗀 뒤에는 취소할 것이 없다.
        out.clear();
        feed.finish(now, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn the_page_mapping_keeps_the_tablet_inside_the_page() {
        let mut feed = feed();
        let now = Instant::now();
        let mut out = Vec::new();
        // 태블릿의 네 모서리가 모두 페이지 안에 들어온다(비율은 `map`의 테스트가 지킨다).
        for (x, y) in [
            (0.0, 0.0),
            (51196.0, 0.0),
            (0.0, 31826.0),
            (51196.0, 31826.0),
        ] {
            feed.push(&sample(x, y, 4000.0, FLAG_TIP), now, &mut out);
        }
        for sample in &out {
            assert!(
                sample.at.x >= -0.01
                    && sample.at.x <= Size::A4.width + 0.01
                    && sample.at.y >= -0.01
                    && sample.at.y <= Size::A4.height + 0.01,
                "페이지 밖이다: {:?}",
                sample.at
            );
        }

        // 페이지가 작아지면 같은 태블릿 좌표가 **다른 점**이 된다(창 크기를 따라간다).
        let before = out[1].at;
        feed.retarget(&deco(), Size::new(100.0, 100.0));
        let mut smaller = Vec::new();
        feed.push(&sample(51196.0, 0.0, 4000.0, FLAG_TIP), now, &mut smaller);
        assert!(smaller[0].at.x <= 100.01, "{:?}", smaller[0].at);
        assert!(smaller[0].at.x < before.x);
    }
}
