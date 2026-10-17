use crate::agent::role::{StepDecision, StepFacts};

pub(crate) fn extract_summary(response: &str) -> Option<String> {
    let start = response.find("<summary>")? + "<summary>".len();
    let rest = &response[start..];
    let end = rest.find("</summary>").unwrap_or(rest.len());
    let body = rest[..end].trim();
    if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    }
}

pub(super) fn decide(facts: StepFacts) -> StepDecision {
    if facts.calls_empty {
        StepDecision::Stop
    } else {
        StepDecision::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::{StepDecision, StepFacts, decide};
    use crate::domain::vo::StepNumber;

    fn facts(calls_empty: bool) -> StepFacts {
        StepFacts {
            step: StepNumber::from(1),
            text_empty: false,
            calls_empty,
            skipped: false,
            non_skip_calls: 0,
        }
    }

    #[test]
    fn stops_when_step_has_no_tool_call() {
        assert!(matches!(decide(facts(true)), StepDecision::Stop));
        assert!(matches!(decide(facts(false)), StepDecision::Continue));
    }
}
