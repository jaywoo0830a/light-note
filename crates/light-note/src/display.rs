//! The frame clock: 60, 120, 180 and 240 Hz.
//!
//! Two jobs, both pure arithmetic so they can be tested without a display:
//!
//! * [`snap_to_supported`] / [`RefreshChoice`] — decide which rate the loop runs
//!   at.  A detected rate is snapped to the nearest supported rate; a tie goes
//!   to the **slower** one (asking for more frames than the panel can show only
//!   wastes power).  An unknown display means 60 Hz.
//! * [`FrameClock`] — hand out **absolute** deadlines (`start + n * period`)
//!   instead of "now + period", so the loop cannot drift, and report lateness
//!   clamped to one period so a machine that just woke from sleep does not try
//!   to catch up with a burst of frames.
//!
//! The waiting itself ([`sleep_precise`]) is the only platform part: a
//! high-resolution waitable timer on Windows, because `thread::sleep` has ~15 ms
//! granularity by default and that is a whole frame at 60 Hz.

use std::time::{Duration, Instant};

/// The rates this app paces itself with.
pub const SUPPORTED_HZ: [u32; 4] = [60, 120, 180, 240];

/// What an unknown display is assumed to be.
pub const DEFAULT_HZ: u32 = 60;

/// The user's choice of frame rate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RefreshChoice {
    /// Follow the display (snapped to a supported rate).
    #[default]
    Auto,
    /// A fixed rate, whatever the display reports.
    Fixed(u32),
}

impl RefreshChoice {
    /// The rate this choice means on a display reporting `detected` Hz.
    pub fn resolve(self, detected: u32) -> u32 {
        match self {
            Self::Auto => snap_to_supported(detected),
            Self::Fixed(hz) => snap_to_supported(hz),
        }
    }

    /// A fixed choice for `hz` (snapped), or [`RefreshChoice::Auto`] if `hz` is 0.
    pub fn from_hz(hz: u32) -> Self {
        if hz == 0 {
            Self::Auto
        } else {
            Self::Fixed(snap_to_supported(hz))
        }
    }
}

/// The supported rate closest to `hz`; ties go to the slower rate, and 0 (an
/// unknown display) means [`DEFAULT_HZ`].
pub fn snap_to_supported(hz: u32) -> u32 {
    if hz == 0 {
        return DEFAULT_HZ;
    }
    let mut best = SUPPORTED_HZ[0];
    let mut best_distance = u32::MAX;
    for candidate in SUPPORTED_HZ {
        let distance = candidate.abs_diff(hz);
        if distance < best_distance {
            best = candidate;
            best_distance = distance;
        }
    }
    best
}

/// One frame at `hz`, in nanoseconds (exact, not "about 4 ms").
pub fn frame_period(hz: u32) -> Duration {
    let hz = snap_to_supported(hz);
    Duration::from_nanos(1_000_000_000 / hz as u64)
}

/// Absolute frame deadlines.
#[derive(Debug)]
pub struct FrameClock {
    hz: u32,
    start: Instant,
    deadline: Instant,
    period: Duration,
}

impl FrameClock {
    pub fn new(hz: u32) -> Self {
        let start = Instant::now();
        Self {
            hz: snap_to_supported(hz),
            start,
            deadline: start,
            period: frame_period(hz),
        }
    }

    /// The instant the schedule started.
    pub fn start(&self) -> Instant {
        self.start
    }

    pub fn hz(&self) -> u32 {
        self.hz
    }

    pub fn period(&self) -> Duration {
        self.period
    }

    /// Changes the rate without moving the current deadline.
    pub fn retarget(&mut self, hz: u32) {
        self.hz = snap_to_supported(hz);
        self.period = frame_period(self.hz);
    }

    /// The next deadline, advanced by exactly one period.
    pub fn next(&mut self) -> Instant {
        self.deadline += self.period;
        self.deadline
    }

    /// How late a tick that arrived at `now` is (never more than one period).
    pub fn lateness(&self, now: Instant) -> Duration {
        now.checked_duration_since(self.deadline)
            .unwrap_or(Duration::ZERO)
            .min(self.period)
    }

    /// Sleeps until `deadline` — called on the ticker thread, never the UI.
    pub fn wait_until(&self, deadline: Instant) {
        if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            sleep_precise(remaining);
        }
    }
}

/// A frame-rate meter: how fast the loop runs, and its worst frame.
#[derive(Debug, Default)]
pub struct FpsMeter {
    window: std::collections::VecDeque<Duration>,
    worst: Duration,
}

impl FpsMeter {
    /// How many frames the window remembers (one second at 240 Hz).
    pub const WINDOW: usize = 240;

    pub fn new() -> Self {
        Self::default()
    }

    /// Records one frame's duration.
    pub fn record(&mut self, frame: Duration) {
        if frame > self.worst {
            self.worst = frame;
        }
        self.window.push_back(frame);
        while self.window.len() > Self::WINDOW {
            self.window.pop_front();
        }
    }

    /// Frames in the window.
    pub fn frames(&self) -> usize {
        self.window.len()
    }

    /// Average rate of the window (0 until something is recorded).
    pub fn fps(&self) -> f32 {
        let total: Duration = self.window.iter().sum();
        if total.is_zero() {
            return 0.0;
        }
        self.window.len() as f32 / total.as_secs_f32()
    }

    /// The worst frame in the window, in milliseconds.
    pub fn worst_ms(&self) -> f32 {
        self.worst.as_secs_f32() * 1000.0
    }

    /// Forgets everything (used when the rate changes).
    pub fn reset(&mut self) {
        self.window.clear();
        self.worst = Duration::ZERO;
    }
}

/// The display's current refresh rate in Hz (0 when it cannot be determined).
pub fn detect_refresh_hz() -> u32 {
    #[cfg(windows)]
    {
        win::detect_refresh_hz()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Sleeps for `duration` without the ~15 ms granularity of `thread::sleep`.
pub fn sleep_precise(duration: Duration) {
    #[cfg(windows)]
    {
        win::sleep_precise(duration);
    }
    #[cfg(not(windows))]
    {
        std::thread::sleep(duration);
    }
}

#[cfg(windows)]
mod win {
    //! The only platform code in this module: reading the compositor's refresh
    //! rate, and waiting with a high-resolution timer.

    use std::time::Duration;

    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo};
    use windows::Win32::System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, SetWaitableTimer,
        TIMER_ALL_ACCESS, WaitForSingleObject,
    };
    use windows::core::PCWSTR;

    /// The compositor knows the real rate (including 120/180/240 Hz panels): it
    /// reports it as the ratio `rateRefresh.uiNumerator / uiDenominator`, so no
    /// QPC arithmetic and no rounding of a period is needed.
    pub fn detect_refresh_hz() -> u32 {
        let mut info = DWM_TIMING_INFO {
            cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
            ..Default::default()
        };
        // A null HWND means "the composition timing of the desktop window".
        if unsafe { DwmGetCompositionTimingInfo(HWND(std::ptr::null_mut()), &mut info) }.is_err() {
            return 0;
        }
        let ratio = info.rateRefresh;
        if ratio.uiNumerator == 0 || ratio.uiDenominator == 0 {
            return 0;
        }
        let hz = ratio.uiNumerator as f64 / ratio.uiDenominator as f64;
        if !hz.is_finite() || hz <= 1.0 || hz > 1000.0 {
            return 0;
        }
        hz.round() as u32
    }

    /// Waits with `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` (Windows 10 1803+),
    /// falling back to a plain sleep when the timer cannot be created.
    pub fn sleep_precise(duration: Duration) {
        let timer = unsafe {
            CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
        };
        let Ok(timer) = timer else {
            std::thread::sleep(duration);
            return;
        };

        // Negative = relative, in 100 ns units.
        let ticks = -((duration.as_nanos() / 100) as i64).max(1);
        if unsafe { SetWaitableTimer(timer, &ticks, 0, None, None, false) }.is_ok() {
            unsafe { WaitForSingleObject(timer, u32::MAX) };
        } else {
            std::thread::sleep(duration);
        }
        let _ = unsafe { CloseHandle(timer) };
    }
}