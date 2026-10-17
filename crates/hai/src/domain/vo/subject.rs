use strum::{Display, EnumString, IntoStaticStr};

/// Which kind of subject one perception row is cached against; the name is the column value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum SubjectKind {
    Handle,
    Url,
}

impl SubjectKind {
    pub fn name(&self) -> &'static str {
        self.into()
    }
}

enum_column!(SubjectKind);

/// What one perception row is about — its `subject` column plus the `subject_kind` that reads it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Subject {
    pub kind: SubjectKind,
    pub value: String,
}

impl Subject {
    /// A platform file handle: `<platform>:<file_unique_id>`.
    pub fn handle(platform: &str, file_unique_id: &str) -> Self {
        Self {
            kind: SubjectKind::Handle,
            value: format!("{platform}:{file_unique_id}"),
        }
    }

    pub fn url(url: &str) -> Self {
        Self {
            kind: SubjectKind::Url,
            value: url.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_key_names_its_platform_and_its_kind() {
        let subject = Subject::handle("telegram", "AgADabc");
        assert_eq!(subject.kind, SubjectKind::Handle);
        assert_eq!(subject.kind.name(), "handle");
        assert_eq!(subject.value, "telegram:AgADabc");

        let url = Subject::url("https://example.com/a");
        assert_eq!(url.kind, SubjectKind::Url);
        assert_eq!(url.value, "https://example.com/a");
    }
}
