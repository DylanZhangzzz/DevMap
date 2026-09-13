//! Explicit owned-fixture diagnostic; no production instrumentation.
use super::*;
use std::time::Instant;

pub(crate) struct Clock {
    diagnostic: &'static str,
    active: bool,
    start: Instant,
    starts: u64,
}
impl Clock {
    pub(super) fn new() -> Self {
        Self {
            diagnostic: "collection-phase/1",
            active: std::env::var("DEVMAP_QUERY_PROFILE_MODE").as_deref()
                == Ok("collection_phases"),
            start: Instant::now(),
            starts: crate::git_process::test_spawn_count() as u64,
        }
    }
    pub(crate) fn relationships() -> Self {
        let mut clock = Self::new();
        clock.diagnostic = "relationship-phase/1";
        clock
    }
    pub(crate) fn mark(&mut self, stage: &str) {
        if !self.active {
            return;
        }
        let wall_us = self.start.elapsed().as_micros();
        let starts = crate::git_process::test_spawn_count() as u64;
        println!(
            "{}",
            serde_json::json!({"diagnostic":self.diagnostic, "stage":stage, "wall_us":wall_us, "git_starts":starts-self.starts})
        );
        self.starts = starts;
        self.start = Instant::now();
    }
}

pub(crate) fn run(workspace: &SourceWorkspace, expected: usize) -> Result<(), DevMapError> {
    let mut previous: Option<DockProjectionContext> = None;
    for iteration in 0..3 {
        let start = Instant::now();
        // This diagnostic deliberately isolates collection with no route targets.
        let next = DockProjectionContext::collect(workspace, &[])?;
        let wall_us = start.elapsed().as_micros();
        assert_eq!(next.worktrees.len(), expected);
        assert_eq!(next.relationships.by_worktree_id.len(), expected);
        if let Some(previous) = &previous {
            assert_eq!(next.worktrees, previous.worktrees);
            assert_eq!(next.topology, previous.topology);
            assert_eq!(next.relationships, previous.relationships);
            assert_eq!(next.configuration_key, previous.configuration_key);
        }
        println!(
            "{}",
            serde_json::json!({"diagnostic":"collection-total/1", "iteration":iteration, "wall_us":wall_us,"worktrees":expected})
        );
        previous = Some(next);
    }
    Ok(())
}
