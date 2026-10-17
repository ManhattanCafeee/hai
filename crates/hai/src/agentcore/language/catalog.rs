use strum::EnumIter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter)]
pub enum TextCatalog {
    Zh,
    En,
}

/// A value that has one form per text catalog. Every literal names both fields, so a new catalog
/// stops compiling here and at every construction site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerCatalog<T> {
    pub zh: T,
    pub en: T,
}

impl<T> PerCatalog<T> {
    pub fn get(self, catalog: TextCatalog) -> T {
        match catalog {
            TextCatalog::Zh => self.zh,
            TextCatalog::En => self.en,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_picks_the_field_matching_the_catalog() {
        let value = PerCatalog {
            zh: "中",
            en: "English",
        };

        assert_eq!(value.get(TextCatalog::Zh), "中");
        assert_eq!(value.get(TextCatalog::En), "English");
    }
}
