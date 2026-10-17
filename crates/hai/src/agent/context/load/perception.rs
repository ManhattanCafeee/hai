use std::collections::{HashMap, HashSet};

use crate::{
    agent::web::WebUrl,
    domain::{
        model::{Attachment, Perception},
        service::Services,
        vo::{AttachmentId, PerceptionId, Subject},
    },
    error::Result,
};

/// The cached analyses one turn's messages can render, indexed for the renderer.
pub struct LoadedPerceptions {
    pub by_attachment: HashMap<AttachmentId, Vec<Perception>>,
    /// A repeat of the same file → the first attachment id carrying its perception.
    pub same_resource_as: HashMap<AttachmentId, AttachmentId>,
    /// URL analyses, for the chapter-opening `<perceptions>` block.
    pub url_items: Vec<Perception>,
}

/// Loads the cached analyses of every attachment plus, on the chapter-opening turn, the analyses of
/// the URLs the messages name.
pub async fn load_perceptions(
    services: &Services,
    attachments: &[Attachment],
    texts: &[String],
    first_turn: bool,
) -> Result<LoadedPerceptions> {
    let subjects: Vec<Subject> = attachments.iter().map(Attachment::subject).collect();
    let rows = services.perception.find_by_subjects(&subjects).await?;

    let mut by_subject: HashMap<Subject, Vec<Perception>> = HashMap::new();
    for row in rows {
        let subject = Subject {
            kind: row.subject_kind,
            value: row.subject.clone(),
        };
        by_subject.entry(subject).or_default().push(row);
    }

    let mut by_attachment: HashMap<AttachmentId, Vec<Perception>> = HashMap::new();
    let mut same_resource_as: HashMap<AttachmentId, AttachmentId> = HashMap::new();
    let mut first_of_subject: HashMap<Subject, AttachmentId> = HashMap::new();

    for att in attachments {
        let subject = att.subject();
        match first_of_subject.get(&subject) {
            Some(first) => {
                same_resource_as.insert(att.id, *first);
            }
            None => {
                first_of_subject.insert(subject.clone(), att.id);
                if let Some(rows) = by_subject.get(&subject) {
                    by_attachment.insert(att.id, rows.clone());
                }
            }
        }
    }

    Ok(LoadedPerceptions {
        by_attachment,
        same_resource_as,
        url_items: if first_turn {
            load_urls(services, texts).await?
        } else {
            Vec::new()
        },
    })
}

async fn load_urls(services: &Services, texts: &[String]) -> Result<Vec<Perception>> {
    // Canonicalize through `WebUrl` so the lookup key matches the one the `read` tool stored.
    let subjects: Vec<Subject> = texts
        .iter()
        .flat_map(|text| crate::util::text::extract_urls(text))
        .filter_map(|raw| WebUrl::parse(&raw).ok())
        .map(|url| Subject::url(url.as_str()))
        .collect();
    let rows = services.perception.find_by_subjects(&subjects).await?;

    let mut seen: HashSet<PerceptionId> = HashSet::new();
    Ok(rows.into_iter().filter(|row| seen.insert(row.id)).collect())
}
