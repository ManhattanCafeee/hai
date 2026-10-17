use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// The name of a knowledge collection, as filed in the `collection` column. A name is never empty:
/// the empty string is how that column spells `Collection::Uncategorised`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CollectionName(String);

impl CollectionName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CollectionName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            Err("a collection name cannot be empty".to_owned())
        } else {
            Ok(Self(value))
        }
    }
}

impl From<CollectionName> for String {
    fn from(value: CollectionName) -> Self {
        value.0
    }
}

impl fmt::Display for CollectionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a document is filed under: a named collection, or the uncategorised bucket the column spells
/// as the empty string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Collection {
    Uncategorised,
    Named(CollectionName),
}

impl Collection {
    /// The column's own spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Uncategorised => "",
            Self::Named(name) => name.as_str(),
        }
    }
}

impl TryFrom<String> for Collection {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            Ok(Self::Uncategorised)
        } else {
            CollectionName::try_from(value).map(Self::Named)
        }
    }
}

impl FromStr for Collection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl From<Collection> for String {
    fn from(value: Collection) -> Self {
        value.as_str().to_owned()
    }
}

impl fmt::Display for Collection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl sqlx::Type<sqlx::Postgres> for Collection {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <&str as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        <&str as sqlx::Type<sqlx::Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for Collection {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let raw = <&str as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)?;
        Ok(Self::try_from(raw.to_owned())?)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for Collection {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<'_, sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}
