use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};

pub(crate) mod catalog;

use crate::agentcore::language::catalog::{PerCatalog, TextCatalog};

/// The language a chat is spoken in; Chinese has a text catalog of its own and every other
/// language falls back to English (`docs/CONTEXT.md` "chat language").
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Display,
    EnumIter,
    EnumString,
    IntoStaticStr,
    Serialize,
    Deserialize,
)]
#[strum(ascii_case_insensitive, serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    #[default]
    Zh,
    En,
    EnGb,
    Ja,
    Ko,
    Es,
    Fr,
    De,
    Pt,
    Ru,
    Ar,
    It,
    Th,
}

impl Language {
    pub fn names(self) -> PerCatalog<LanguageName> {
        match self {
            Self::Zh => PerCatalog {
                zh: LanguageName::new("中文"),
                en: LanguageName::new("Chinese"),
            },
            Self::En => PerCatalog {
                zh: LanguageName::new("英语"),
                en: LanguageName::new("English"),
            },
            Self::EnGb => PerCatalog {
                zh: LanguageName::new("英式英语"),
                en: LanguageName::new("British English"),
            },
            Self::Ja => PerCatalog {
                zh: LanguageName::new("日语"),
                en: LanguageName::new("Japanese"),
            },
            Self::Ko => PerCatalog {
                zh: LanguageName::new("韩语"),
                en: LanguageName::new("Korean"),
            },
            Self::Es => PerCatalog {
                zh: LanguageName::new("西班牙语"),
                en: LanguageName::new("Spanish"),
            },
            Self::Fr => PerCatalog {
                zh: LanguageName::new("法语"),
                en: LanguageName::new("French"),
            },
            Self::De => PerCatalog {
                zh: LanguageName::new("德语"),
                en: LanguageName::new("German"),
            },
            Self::Pt => PerCatalog {
                zh: LanguageName::new("葡萄牙语"),
                en: LanguageName::new("Portuguese"),
            },
            Self::Ru => PerCatalog {
                zh: LanguageName::new("俄语"),
                en: LanguageName::new("Russian"),
            },
            Self::Ar => PerCatalog {
                zh: LanguageName::new("阿拉伯语"),
                en: LanguageName::new("Arabic"),
            },
            Self::It => PerCatalog {
                zh: LanguageName::new("意大利语"),
                en: LanguageName::new("Italian"),
            },
            Self::Th => PerCatalog {
                zh: LanguageName::new("泰语"),
                en: LanguageName::new("Thai"),
            },
        }
    }

    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// The code a client platform matches a client's language by: two-letter ISO 639-1. A regional
    /// variant has none of its own, so it is served by its base language — `en-gb` reads `en`.
    pub fn client_code(self) -> &'static str {
        let tag = self.as_str();
        tag.split_once('-').map_or(tag, |(base, _)| base)
    }

    pub fn text_catalog(self) -> TextCatalog {
        match self {
            Self::Zh => TextCatalog::Zh,
            _ => TextCatalog::En,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageName(&'static str);

impl LanguageName {
    const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// serde and strum read separate attributes, so the wire names are pinned here.
    #[test]
    fn wire_form_and_display_names_cover_every_variant() {
        for language in Language::iter() {
            let wire = serde_json::to_value(language).expect("serializable");
            assert_eq!(wire.as_str(), Some(language.as_str()), "{language:?}");
            assert_eq!(language.as_str().parse::<Language>().ok(), Some(language));
            assert_eq!(
                language.as_str().to_uppercase().parse::<Language>().ok(),
                Some(language),
                "{language:?}"
            );
            for catalog in TextCatalog::iter() {
                assert!(
                    !language.names().get(catalog).as_str().is_empty(),
                    "{language:?} / {catalog:?}"
                );
            }
        }
    }

    /// A client platform matches a two-letter ISO 639-1 code, so every variant carries one and a
    /// regional variant carries its base language's.
    #[test]
    fn every_client_code_is_a_two_letter_lowercase_code() {
        for language in Language::iter() {
            let code = language.client_code();
            assert_eq!(code.len(), 2, "{language:?} / {code}");
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase()),
                "{language:?} / {code}"
            );
        }
        assert_eq!(Language::EnGb.client_code(), "en");
    }

    #[test]
    fn absent_configuration_speaks_chinese() {
        assert_eq!(Language::default(), Language::Zh);
    }

    /// Every catalog is one some language reads: text in a catalog nothing maps to is unreachable,
    /// so a new catalog has to route a language to it.
    #[test]
    fn every_catalog_is_read_by_some_language() {
        for catalog in TextCatalog::iter() {
            assert!(
                Language::iter().any(|language| language.text_catalog() == catalog),
                "{catalog:?}"
            );
        }
    }
}
