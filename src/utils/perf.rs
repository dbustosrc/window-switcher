#[cfg(feature = "perf")]
pub struct PerfSpan {
    name: &'static str,
    started: std::time::Instant,
}

#[cfg(feature = "perf")]
impl PerfSpan {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            started: std::time::Instant::now(),
        }
    }
}

#[cfg(feature = "perf")]
impl Drop for PerfSpan {
    fn drop(&mut self) {
        log::info!(
            target: "window_switcher::perf",
            "{} {:.3} ms",
            self.name,
            self.started.elapsed().as_secs_f64() * 1_000.0
        );
    }
}

#[cfg(not(feature = "perf"))]
pub struct PerfSpan;

#[cfg(not(feature = "perf"))]
impl PerfSpan {
    #[inline(always)]
    pub const fn new(_name: &'static str) -> Self {
        Self
    }
}
