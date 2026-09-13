//! Buffered, opt-in test diagnostics. No production instrumentation.
use std::time::Instant;

pub(crate) struct Clock {
    diagnostic: &'static str,
    active: bool,
    start: Instant,
    starts: usize,
    rows: Vec<(&'static str, u128, usize)>,
}
impl Clock {
    pub(super) fn new() -> Self {
        Self::selected("boundary_phases", "query-boundary-phase/1", true)
    }
    pub(super) fn projection(enabled: bool) -> Self {
        Self::selected("projection_phases", "query-projection-phase/1", enabled)
    }
    pub(crate) fn storage(enabled: bool) -> Self {
        Self::selected("storage_phases", "query-storage-phase/1", enabled)
    }
    pub(crate) fn observer() -> Self {
        Self::selected("observer_phases", "query-observer-phase/1", true)
    }
    pub(crate) fn inventory() -> Self {
        Self::selected("inventory_phases", "query-inventory-phase/1", true)
    }
    fn selected(mode: &str, diagnostic: &'static str, enabled: bool) -> Self {
        Self {
            diagnostic,
            active: enabled && std::env::var("DEVMAP_QUERY_PROFILE_MODE").as_deref() == Ok(mode),
            start: Instant::now(),
            starts: crate::git_process::test_spawn_count(),
            rows: Vec::new(),
        }
    }
    pub(crate) fn mark(&mut self, stage: &'static str) {
        if self.active {
            let starts = crate::git_process::test_spawn_count();
            self.rows.push((
                stage,
                self.start.elapsed().as_micros(),
                starts - self.starts,
            ));
            self.starts = starts;
            self.start = Instant::now();
        }
    }
    pub(crate) fn finish(self) {
        for (stage, wall_us, git_starts) in self.rows {
            println!(
                "{}",
                serde_json::json!({
                    "diagnostic":self.diagnostic, "stage":stage,
                    "wall_us":wall_us, "git_starts":git_starts,
                })
            );
        }
    }
}
