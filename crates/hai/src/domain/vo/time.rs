use serde::{Deserialize, Serialize};
use sqlx::{
    postgres::{PgArgumentBuffer, PgArguments, PgValueRef, Postgres},
    query::QueryAs,
};

/// A window on a timestamp column; either end may be open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimeRange {
    pub since: Option<jiff::Timestamp>,
    pub until: Option<jiff::Timestamp>,
}

impl TimeRange {
    pub fn new(since: Option<jiff::Timestamp>, until: Option<jiff::Timestamp>) -> Self {
        Self { since, until }
    }

    /// Binds both ends in the order a query reads them: `since`, then `until`.
    pub fn bind<'q, O>(
        self,
        query: QueryAs<'q, Postgres, O, PgArguments>,
    ) -> QueryAs<'q, Postgres, O, PgArguments> {
        query
            .bind(self.since.map(jiff_sqlx::Timestamp::from))
            .bind(self.until.map(jiff_sqlx::Timestamp::from))
    }
}

/// A task's repeat step: a positive number of seconds, so a repeating task cannot exist without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepeatInterval(std::num::NonZeroI64);

impl RepeatInterval {
    pub fn secs(&self) -> i64 {
        self.0.get()
    }

    pub fn as_duration(&self) -> jiff::SignedDuration {
        jiff::SignedDuration::from_secs(self.secs())
    }
}

impl TryFrom<i64> for RepeatInterval {
    type Error = String;

    fn try_from(secs: i64) -> Result<Self, Self::Error> {
        std::num::NonZeroI64::new(secs)
            .map(Self)
            .ok_or_else(|| "a repeat interval must be a positive number of seconds".to_owned())
    }
}

impl sqlx::Type<sqlx::Postgres> for RepeatInterval {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <i64 as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        <i64 as sqlx::Type<sqlx::Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for RepeatInterval {
    fn decode(value: PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let secs = <i64 as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)?;
        Ok(Self::try_from(secs).map_err(|e| format!("{e}: {secs}"))?)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for RepeatInterval {
    fn encode_by_ref(
        &self,
        buf: &mut PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <i64 as sqlx::Encode<'_, sqlx::Postgres>>::encode_by_ref(&self.secs(), buf)
    }
}
