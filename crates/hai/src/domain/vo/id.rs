use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    (pg $vis:vis struct $name:ident(pub $inner:ty);) => {
        id_type!($vis struct $name(pub $inner););

        impl sqlx::Type<sqlx::Postgres> for $name {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <$inner as sqlx::Type<sqlx::Postgres>>::type_info()
            }

            fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
                <$inner as sqlx::Type<sqlx::Postgres>>::compatible(ty)
            }
        }

        impl sqlx::postgres::PgHasArrayType for $name {
            fn array_type_info() -> sqlx::postgres::PgTypeInfo {
                <$inner as sqlx::postgres::PgHasArrayType>::array_type_info()
            }

            fn array_compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
                <$inner as sqlx::postgres::PgHasArrayType>::array_compatible(ty)
            }
        }

        impl<'r> sqlx::Decode<'r, sqlx::Postgres> for $name {
            fn decode(
                value: sqlx::postgres::PgValueRef<'r>,
            ) -> std::result::Result<Self, sqlx::error::BoxDynError> {
                <$inner as sqlx::Decode<'r, sqlx::Postgres>>::decode(value).map($name)
            }
        }

        impl sqlx::Encode<'_, sqlx::Postgres> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut sqlx::postgres::PgArgumentBuffer,
            ) -> std::result::Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <$inner as sqlx::Encode<'_, sqlx::Postgres>>::encode_by_ref(&self.0, buf)
            }
        }
    };

    ($vis:vis struct $name:ident(pub $inner:ty);) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
        #[serde(transparent)]
        $vis struct $name(pub $inner);

        impl From<$inner> for $name {
            fn from(v: $inner) -> Self { Self(v) }
        }
        impl From<$name> for $inner {
            fn from(v: $name) -> Self { v.0 }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
        impl $name {
            pub fn raw_ids(ids: &[Self]) -> Vec<$inner> {
                ids.iter().map(|id| id.0).collect()
            }
        }
    };
}

id_type!(
    pg pub struct ChatId(pub i64);
);
id_type!(
    pg pub struct MessageId(pub i64);
);
id_type!(
    pg pub struct AccountId(pub i64);
);
id_type!(
    pg pub struct TopicId(pub uuid::Uuid);
);
id_type!(
    pg pub struct MemoryId(pub uuid::Uuid);
);
id_type!(
    pg pub struct AttachmentId(pub uuid::Uuid);
);
id_type!(
    pg pub struct PerceptionId(pub uuid::Uuid);
);
id_type!(
    pg pub struct IdentityId(pub uuid::Uuid);
);
id_type!(
    pg pub struct KnowledgeDocumentId(pub uuid::Uuid);
);
id_type!(
    pg pub struct KnowledgeChunkId(pub uuid::Uuid);
);
id_type!(
    pg pub struct ScheduledTaskId(pub uuid::Uuid);
);
id_type!(
    pub struct TurnNumber(pub usize);
);
id_type!(
    pub struct StepNumber(pub usize);
);

/// Namespace for attachment occurrence ids; never reused by another identity.
const ATTACHMENT_NS: Uuid = Uuid::from_u128(0x8f4b_2c17_9e60_4a3d_b5c8_1e7f_9d02_6a34);

/// The occurrence id of one attachment of a platform message: the same delivered message part
/// keeps its id when the message is stored again.
pub fn attachment_id(
    platform: &str,
    chat_id: ChatId,
    external_id: &str,
    ordinal: i32,
) -> AttachmentId {
    AttachmentId(Uuid::new_v5(
        &ATTACHMENT_NS,
        format!("{platform}:{}:{external_id}:{ordinal}", chat_id.0).as_bytes(),
    ))
}

/// The key one channel instance is stored under: the platform's name and the instance key the config
/// gave, spelled once because the column, the query and the logger all read it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelKey(String);

impl ChannelKey {
    pub fn new(platform: &str, key: &str) -> Self {
        Self(format!("{platform}:{key}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ChannelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl sqlx::Type<sqlx::Postgres> for ChannelKey {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        <String as sqlx::Type<sqlx::Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for ChannelKey {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> std::result::Result<Self, sqlx::error::BoxDynError> {
        <String as sqlx::Decode<'r, sqlx::Postgres>>::decode(value).map(Self)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for ChannelKey {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> std::result::Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <String as sqlx::Encode<'_, sqlx::Postgres>>::encode_by_ref(&self.0, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_key_names_its_platform() {
        let key = ChannelKey::new("telegram", "main");
        assert_eq!(key.as_str(), "telegram:main");
        assert_eq!(key.to_string(), "telegram:main");
    }
}
