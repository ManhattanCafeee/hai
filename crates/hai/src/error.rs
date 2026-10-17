use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AppError {
    #[error("{0}")]
    Message(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Toml(#[from] toml::ser::Error),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    /// A platform's own wire failure, mapped at that platform's boundary: no vendor type reaches
    /// this enum, so a second platform adds no variant here.
    #[error("{platform}: {source}")]
    Adapter {
        platform: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error(transparent)]
    Jiff(#[from] jiff::Error),
    #[error(transparent)]
    Config(#[from] config::ConfigError),
}

impl AppError {
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }

    /// One platform's failure with the platform named and its own error kept as the source.
    pub fn adapter(
        platform: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Adapter {
            platform,
            source: Box::new(source),
        }
    }
}

macro_rules! bail {
    ($err:expr) => {
        return Err($err.into())
    };
}

pub(crate) use bail;

pub type Result<T, E = AppError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_the_message() {
        assert_eq!(AppError::message("bad url").to_string(), "bad url");
    }

    #[test]
    fn typed_sources_keep_their_type_and_text() {
        let err = AppError::from(std::io::Error::other("disk"));
        assert!(matches!(err, AppError::Io(_)));
        assert_eq!(err.to_string(), "disk");
    }
}
