use serde::{Deserialize, Serialize};

/// Chapter-level scalars persisted alongside the conversation snapshot; reopening a chapter
/// (`start_new_chapter`) resets all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContextMeta {
    /// Context occupancy: prompt tokens of the last successful Turn. The overflow trigger reads
    /// it.
    pub tokens: u32,
    /// Successful Turns in the chapter; `Success` and `Steered` advance it, `Failed` does not.
    pub turn_count: u64,
    /// Steps accumulated in the chapter; reported as `WrapUpCompleted.step_count`.
    pub step_count: u64,
}

impl ContextMeta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&mut self, step_count: usize, last_prompt_tokens: u32) {
        self.tokens = last_prompt_tokens;
        self.turn_count += 1;
        self.step_count += step_count as u64;
    }

    pub fn is_non_empty(&self) -> bool {
        self.turn_count > 0
    }
}
