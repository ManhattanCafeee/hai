//! What the Telegram Bot API takes in one send: the forms it serves there, and the ceiling on a
//! single upload. The send tool's surface is pruned from these, so nothing asks Telegram for a form
//! it does not have.

use crate::agent::channel::{BuildCapability, OutboundField, OutboundForm, OutboundKind};

/// The Bot API's ceiling for one upload.
pub(super) const MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;

/// What one of this platform's bots can put in a chat, and what each form takes there.
pub(super) const TELEGRAM_FORMS: [OutboundForm; 3] = [
    OutboundForm {
        kind: OutboundKind::Text,
        fields: &[OutboundField {
            name: "content",
            description: "What to say",
        }],
        requires: BuildCapability::None,
    },
    OutboundForm {
        kind: OutboundKind::Voice,
        fields: &[
            OutboundField {
                name: "prompt",
                description: "How the message should be spoken",
            },
            OutboundField {
                name: "content",
                description: "The line that goes with it",
            },
        ],
        requires: BuildCapability::Speech,
    },
    OutboundForm {
        kind: OutboundKind::File,
        fields: &[
            OutboundField {
                name: "source",
                description: "An existing thing to send: `workspace://<path>`, `file://<path>`, a \
                              bare path, or an `http(s)://` URL",
            },
            OutboundField {
                name: "content",
                description: "The line that goes with it",
            },
        ],
        requires: BuildCapability::None,
    },
];
