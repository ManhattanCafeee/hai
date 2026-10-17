use crate::agentcore::{Language, LanguageName, PerCatalog};

/// Every reply the host composes itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HostCopy {
    Unauthorized,
    Greeting,
    /// What `/lang` did. None of them carries a language: the reply names the effective one.
    LanguageCurrent,
    LanguageSet,
    LanguageReset,
    LanguageUnsupported,
    /// The agent's own status block, which is absent when the agent is not running.
    Status(Option<String>),
    Unread,
}

impl HostCopy {
    /// `language` is both the language the reply is written in and, for the language replies, the
    /// language they are about.
    pub(crate) fn render(self, language: Language) -> String {
        let catalog = language.text_catalog();
        let name = language.names().get(catalog);
        match self {
            Self::Unauthorized => UNAUTHORIZED.get(catalog).into(),
            Self::Greeting => GREETING.get(catalog).into(),
            Self::LanguageCurrent => (LANGUAGE_CURRENT.get(catalog))(name),
            Self::LanguageSet => (LANGUAGE_SET.get(catalog))(name),
            Self::LanguageReset => (LANGUAGE_RESET.get(catalog))(name),
            Self::LanguageUnsupported => LANGUAGE_UNSUPPORTED.get(catalog).into(),
            Self::Status(None) => STATUS_EMPTY.get(catalog).into(),
            Self::Status(Some(body)) => body,
            Self::Unread => UNREAD.get(catalog).into(),
        }
    }
}

const UNAUTHORIZED: PerCatalog<&'static str> = PerCatalog {
    zh: "你没有使用这个机器人的权限。如有疑问，请联系机器人所有者。",
    en: concat!(
        "You are not authorized to use this bot. ",
        "If you think this is a mistake, please contact the bot owner."
    ),
};

const GREETING: PerCatalog<&'static str> = PerCatalog {
    zh: "你好！在的，有事就说。",
    en: "Hello! I am here — talk to me whenever.",
};

const STATUS_EMPTY: PerCatalog<&'static str> = PerCatalog {
    zh: "状态  未运行",
    en: "status  not running",
};

const UNREAD: PerCatalog<&'static str> = PerCatalog {
    zh: "无法识别的参数。",
    en: "Unknown argument.",
};

const LANGUAGE_UNSUPPORTED: PerCatalog<&'static str> = PerCatalog {
    zh: "无法识别的语言代码。",
    en: "Unknown language code.",
};

const LANGUAGE_CURRENT: PerCatalog<fn(LanguageName) -> String> = PerCatalog {
    zh: language_current_zh,
    en: language_current_en,
};

const LANGUAGE_SET: PerCatalog<fn(LanguageName) -> String> = PerCatalog {
    zh: language_set_zh,
    en: language_set_en,
};

const LANGUAGE_RESET: PerCatalog<fn(LanguageName) -> String> = PerCatalog {
    zh: language_reset_zh,
    en: language_reset_en,
};

fn language_current_zh(name: LanguageName) -> String {
    format!("当前语言：{}", name.as_str())
}

fn language_current_en(name: LanguageName) -> String {
    format!("Current language: {}", name.as_str())
}

fn language_set_zh(name: LanguageName) -> String {
    format!("语言已设为{}。", name.as_str())
}

fn language_set_en(name: LanguageName) -> String {
    format!("Language set to {}.", name.as_str())
}

fn language_reset_zh(name: LanguageName) -> String {
    format!("已恢复默认：{}。", name.as_str())
}

fn language_reset_en(name: LanguageName) -> String {
    format!("Back to the default: {}.", name.as_str())
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// A reply naming a language names it the way the catalog it is written in spells it.
    #[test]
    fn language_replies_name_the_language_in_their_own_set() {
        let naming = [
            HostCopy::LanguageCurrent,
            HostCopy::LanguageSet,
            HostCopy::LanguageReset,
        ];
        for language in Language::iter() {
            let name = language.names().get(language.text_catalog()).as_str();
            for copy in naming.clone() {
                assert!(
                    copy.clone().render(language).contains(name),
                    "{copy:?} / {language:?} does not name {name}"
                );
            }
        }
    }
}
