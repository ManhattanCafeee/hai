use serde::{Deserialize, Serialize};

/// A command the user hands the agent: the instruction the command table parsed, carried as it is by
/// the `Command` entry and read by the wake it produces. One vocabulary, from the platform's parse to
/// the turn that acts on it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub enum AgentCommand {
    OrganizeMemory,
    Explain(Option<String>),
    Digest(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(command: AgentCommand) -> AgentCommand {
        let wire = serde_json::to_value(&command).expect("serialize");
        serde_json::from_value(wire).expect("deserialize")
    }

    #[test]
    fn a_command_round_trips_through_its_wire_form() {
        for command in [
            AgentCommand::OrganizeMemory,
            AgentCommand::Explain(None),
            AgentCommand::Explain(Some("borrow checker".to_owned())),
            AgentCommand::Digest(7),
        ] {
            assert_eq!(round_trip(command.clone()), command);
        }
    }

    #[test]
    fn the_wire_form_names_the_command_and_its_argument() {
        assert_eq!(
            serde_json::to_value(AgentCommand::Digest(7)).expect("serialize"),
            serde_json::json!({ "command": "digest", "args": 7 })
        );
        assert_eq!(
            serde_json::to_value(AgentCommand::Explain(None)).expect("serialize"),
            serde_json::json!({ "command": "explain", "args": null })
        );
        assert_eq!(
            serde_json::to_value(AgentCommand::OrganizeMemory).expect("serialize"),
            serde_json::json!({ "command": "organize_memory" })
        );
    }

    #[test]
    fn an_unknown_command_or_argument_is_an_error() {
        assert!(
            serde_json::from_value::<AgentCommand>(serde_json::json!({ "command": "sing" }))
                .is_err(),
            "an unknown command must not decode"
        );
        assert!(
            serde_json::from_value::<AgentCommand>(
                serde_json::json!({ "command": "digest", "args": "soon" })
            )
            .is_err(),
            "a digest without a number of days must not decode"
        );
    }
}
