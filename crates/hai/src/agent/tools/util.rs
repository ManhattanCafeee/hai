use serde::Deserialize;
use serde_json::Value;

use crate::agentcore::tool::{ToolError, tool_err};

pub const MAX_RESULTS: u32 = 20;

pub fn bounded_limit(limit: Option<u32>, default: u32) -> u32 {
    limit.unwrap_or(default).clamp(1, MAX_RESULTS)
}

pub fn required<T>(value: Option<T>, field: &str, op: &str) -> Result<T, ToolError> {
    value.ok_or_else(|| tool_err(format!("op `{op}` requires `{field}`")))
}

/// Lenient `Option<Vec<T>>` of integers: the model may send an array, a bare number, or a `"1,2"`
/// string. Fractions and unparsable values are rejected rather than rounded or dropped.
pub fn deserialize_option_lenient_i64_vec<'de, D, T>(d: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: From<i64>,
{
    let value = Value::deserialize(d)?;
    let raw: Option<Vec<i64>> = match value {
        Value::Null => None,
        Value::Array(items) => Some(items.iter().map(integer).collect::<Result<_, _>>()?),
        Value::String(text) if text.contains(',') => Some(
            text.split(',')
                .map(|part| part.trim().parse::<i64>().map_err(serde::de::Error::custom))
                .collect::<Result<_, _>>()?,
        ),
        scalar => Some(vec![integer(&scalar)?]),
    };
    Ok(raw.map(|ids| ids.into_iter().map(T::from).collect()))
}

fn integer<E: serde::de::Error>(value: &Value) -> Result<i64, E> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| serde::de::Error::custom(format!("expected an integer, got {number}"))),
        Value::String(text) => text.trim().parse::<i64>().map_err(serde::de::Error::custom),
        other => Err(serde::de::Error::custom(format!(
            "expected an integer, got {other}"
        ))),
    }
}

pub fn deserialize_option_lenient_u64<'de, D>(d: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(d)?;
    match value {
        Value::Null => Ok(None),
        Value::String(text) if text.trim().is_empty() => Ok(None),
        scalar => integer_u64(&scalar).map(Some),
    }
}

fn integer_u64<E: serde::de::Error>(value: &Value) -> Result<u64, E> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom(format!("expected an integer, got {number}"))),
        Value::String(text) => text.trim().parse::<u64>().map_err(serde::de::Error::custom),
        other => Err(serde::de::Error::custom(format!(
            "expected an integer, got {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Deserialize)]
    struct Lenient {
        #[serde(
            default,
            deserialize_with = "super::deserialize_option_lenient_i64_vec"
        )]
        ids: Option<Vec<i64>>,
    }

    fn ids(value: serde_json::Value) -> Option<Vec<i64>> {
        serde_json::from_value::<Lenient>(json!({ "ids": value }))
            .expect("lenient")
            .ids
    }

    #[test]
    fn lenient_i64_accepts_array_scalar_and_comma_string() {
        assert_eq!(ids(json!([1, 2])), Some(vec![1, 2]));
        assert_eq!(ids(json!(7)), Some(vec![7]));
        assert_eq!(ids(json!("7")), Some(vec![7]));
        assert_eq!(ids(json!("7, 8")), Some(vec![7, 8]));
        assert_eq!(ids(json!(null)), None);
    }

    #[test]
    fn lenient_i64_rejects_fractions_and_junk() {
        assert!(serde_json::from_value::<Lenient>(json!({ "ids": 1.5 })).is_err());
        assert!(serde_json::from_value::<Lenient>(json!({ "ids": [1.5] })).is_err());
        assert!(serde_json::from_value::<Lenient>(json!({ "ids": "a,b" })).is_err());
        assert!(serde_json::from_value::<Lenient>(json!({ "ids": true })).is_err());
    }
}
