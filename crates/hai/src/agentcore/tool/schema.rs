//! Provider-compatibility normalization for tool-argument schemas: the single entry point every
//! tool (MCP included) passes through before going on the wire.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

/// Recursion limit for combinator nesting; anything deeper is treated as pathological and
/// degraded to an empty object schema.
const MAX_DEPTH: u8 = 4;

pub fn normalize_root(tool: &str, schema: Value) -> Value {
    let defs = schema.get("$defs").cloned();
    let mut normalized = match object_root(&schema, &schema, MAX_DEPTH) {
        Some(object) => object,
        None => {
            tracing::warn!(
                tool,
                "tool schema has no object root; normalized to an empty object schema"
            );
            json!({ "type": "object", "properties": {} })
        }
    };
    // A root `$ref` resolves away the object that carried `$defs`; keep it so nested `#/$defs/…`
    // references in properties stay resolvable.
    if let (Some(defs), Some(object)) = (defs, normalized.as_object_mut()) {
        object.insert("$defs".to_owned(), defs);
    }
    normalized
}

fn object_root(root: &Value, node: &Value, depth: u8) -> Option<Value> {
    if depth == 0 {
        return None;
    }
    let node = resolve(root, node)?;
    let branches = branches(node);
    if branches.is_empty() {
        return object_shell(node);
    }
    merge(root, node, &branches, depth)
}

fn object_shell(node: &Value) -> Option<Value> {
    if !is_object(node) && node.get("properties").is_none() {
        return None;
    }
    let mut object = node.clone();
    if let Some(fields) = object.as_object_mut() {
        for key in ["allOf", "oneOf", "anyOf"] {
            fields.remove(key);
        }
        fields.insert("type".to_owned(), json!("object"));
    }
    Some(object)
}

fn is_object(node: &Value) -> bool {
    node.get("type").and_then(Value::as_str) == Some("object")
}

fn resolve<'a>(root: &'a Value, node: &'a Value) -> Option<&'a Value> {
    let mut current = node;
    for _ in 0..4 {
        let Some(reference) = current.get("$ref").and_then(Value::as_str) else {
            return Some(current);
        };
        current = root.pointer(reference.strip_prefix('#')?)?;
    }
    None
}

/// The root's combinator members. Only the first non-empty one is read; a schema carrying two is
/// not one these tools emit, so the second is dropped with a warning rather than silently.
fn branches(node: &Value) -> Vec<Value> {
    let mut found = Vec::new();
    for key in ["allOf", "oneOf", "anyOf"] {
        let Some(items) = node
            .get(key)
            .and_then(Value::as_array)
            .filter(|i| !i.is_empty())
        else {
            continue;
        };
        if !found.is_empty() {
            tracing::warn!(combinator = key, "a second root combinator is dropped");
            continue;
        }
        found = items.clone();
    }
    found
}

fn merge(root: &Value, node: &Value, branches: &[Value], depth: u8) -> Option<Value> {
    let union_required = node.get("allOf").is_some();
    let own = object_shell(node);
    let mut properties = Map::new();
    let mut required_sets: Vec<HashSet<String>> = Vec::with_capacity(branches.len() + 1);
    for member in own.iter().chain(branches.iter()) {
        let Some(object) = object_root(root, member, depth - 1) else {
            tracing::warn!("tool schema branch is not an object; dropped");
            continue;
        };
        required_sets.push(required_names(&object));
        let Some(fields) = object.get("properties").and_then(Value::as_object) else {
            continue;
        };
        for (name, property) in fields {
            match properties.get_mut(name) {
                Some(existing) => merge_property(existing, property),
                None => {
                    properties.insert(name.clone(), property.clone());
                }
            }
        }
    }
    if properties.is_empty() {
        return None;
    }

    let required: Vec<String> = if union_required {
        let mut all: Vec<String> = required_sets.iter().flatten().cloned().collect();
        all.sort();
        all.dedup();
        all
    } else {
        required_sets
            .iter()
            .fold(None::<HashSet<String>>, |acc, set| {
                Some(match acc {
                    Some(prev) => prev.intersection(set).cloned().collect(),
                    None => set.clone(),
                })
            })
            .map(|set| {
                let mut names: Vec<String> = set.into_iter().collect();
                names.sort();
                names
            })
            .unwrap_or_default()
    };

    let mut object = Map::new();
    object.insert("type".to_owned(), json!("object"));
    if let Some(description) = node.get("description") {
        object.insert("description".to_owned(), description.clone());
    }
    object.insert("properties".to_owned(), Value::Object(properties));
    if !required.is_empty() {
        object.insert("required".to_owned(), json!(required));
    }
    Some(Value::Object(object))
}

fn merge_property(existing: &mut Value, incoming: &Value) {
    let mut values: Vec<Value> = enum_values(existing);
    for value in enum_values(incoming) {
        if !values.contains(&value) {
            values.push(value);
        }
    }
    if values.is_empty() {
        return;
    }
    if let Some(object) = existing.as_object_mut() {
        object.remove("const");
        if all_strings(&values) {
            object.insert("type".to_owned(), json!("string"));
        }
        object.insert("enum".to_owned(), Value::Array(values));
    }
}

fn enum_values(node: &Value) -> Vec<Value> {
    if let Some(value) = node.get("const") {
        return vec![value.clone()];
    }
    node.get("enum")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn all_strings(values: &[Value]) -> bool {
    values.iter().all(Value::is_string)
}

fn required_names(object: &Value) -> HashSet<String> {
    object
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_root_passes_through() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        assert_eq!(normalize_root("t", schema.clone()), schema);
    }

    #[test]
    fn object_root_resolves_local_ref() {
        let schema = json!({
            "$defs": {"Args": {"type": "object", "properties": {"x": {"type": "string"}}}},
            "$ref": "#/$defs/Args"
        });
        assert_eq!(
            normalize_root("t", schema),
            json!({
                "type": "object",
                "properties": {"x": {"type": "string"}},
                "$defs": {"Args": {"type": "object", "properties": {"x": {"type": "string"}}}}
            })
        );
    }

    #[test]
    fn nested_refs_keep_the_root_defs() {
        let schema = json!({
            "$defs": {
                "Args": {"type": "object", "properties": {"inner": {"$ref": "#/$defs/Inner"}}},
                "Inner": {"type": "object", "properties": {"n": {"type": "integer"}}}
            },
            "$ref": "#/$defs/Args"
        });
        assert_eq!(
            normalize_root("t", schema),
            json!({
                "type": "object",
                "properties": {"inner": {"$ref": "#/$defs/Inner"}},
                "$defs": {
                    "Args": {"type": "object", "properties": {"inner": {"$ref": "#/$defs/Inner"}}},
                    "Inner": {"type": "object", "properties": {"n": {"type": "integer"}}}
                }
            })
        );
    }

    #[test]
    fn missing_type_with_properties_becomes_object() {
        assert_eq!(
            normalize_root("t", json!({"properties": {"x": {"type": "string"}}})),
            json!({"type": "object", "properties": {"x": {"type": "string"}}})
        );
    }

    #[test]
    fn one_of_with_const_discriminator_merges() {
        let schema = json!({
            "oneOf": [
                {"type": "object", "properties": {"op": {"const": "create"}, "title": {"type": "string"}},
                 "required": ["op", "title"]},
                {"type": "object", "properties": {"op": {"const": "delete"}, "id": {"type": "string"}},
                 "required": ["op", "id"]}
            ]
        });
        assert_eq!(
            normalize_root("t", schema),
            json!({
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": ["create", "delete"]},
                    "title": {"type": "string"},
                    "id": {"type": "string"}
                },
                "required": ["op"]
            })
        );
    }

    #[test]
    fn merged_enum_values_stay_unique_across_branch_order() {
        let schema = json!({
            "oneOf": [
                {"type": "object", "properties": {"mode": {"enum": ["a", "b"]}}},
                {"type": "object", "properties": {"mode": {"enum": ["b", "a"]}}}
            ]
        });
        assert_eq!(
            normalize_root("t", schema),
            json!({
                "type": "object",
                "properties": {"mode": {"type": "string", "enum": ["a", "b"]}}
            })
        );
    }

    #[test]
    fn any_of_without_discriminator_merges_union() {
        let schema = json!({
            "anyOf": [
                {"type": "object", "properties": {"a": {"type": "string"}}},
                {"type": "object", "properties": {"b": {"type": "integer"}}}
            ]
        });
        assert_eq!(
            normalize_root("t", schema),
            json!({
                "type": "object",
                "properties": {"a": {"type": "string"}, "b": {"type": "integer"}}
            })
        );
    }

    #[test]
    fn all_of_unions_required() {
        let schema = json!({
            "allOf": [
                {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]},
                {"type": "object", "properties": {"b": {"type": "string"}}, "required": ["b"]}
            ]
        });
        assert_eq!(
            normalize_root("t", schema),
            json!({
                "type": "object",
                "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
                "required": ["a", "b"]
            })
        );
    }

    #[test]
    fn non_object_branch_keeps_the_rest() {
        let schema = json!({
            "anyOf": [
                {"type": "null"},
                {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]}
            ]
        });
        assert_eq!(
            normalize_root("t", schema),
            json!({
                "type": "object",
                "properties": {"q": {"type": "string"}},
                "required": ["q"]
            })
        );

        let missing_ref = json!({
            "anyOf": [
                {"$ref": "#/$defs/Absent"},
                {"type": "object", "properties": {"x": {"type": "string"}}}
            ]
        });
        assert_eq!(
            normalize_root("t", missing_ref),
            json!({"type": "object", "properties": {"x": {"type": "string"}}})
        );
    }

    #[test]
    fn object_root_with_own_combinator_merges_and_strips_it() {
        let one_of = json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "oneOf": [
                {"type": "object", "properties": {"b": {"type": "integer"}}, "required": ["b"]}
            ]
        });
        assert_eq!(
            normalize_root("t", one_of),
            json!({
                "type": "object",
                "properties": {"a": {"type": "string"}, "b": {"type": "integer"}}
            })
        );

        let all_of = json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "allOf": [
                {"type": "object", "properties": {"b": {"type": "integer"}}, "required": ["b"]}
            ]
        });
        assert_eq!(
            normalize_root("t", all_of),
            json!({
                "type": "object",
                "properties": {"a": {"type": "string"}, "b": {"type": "integer"}},
                "required": ["a", "b"]
            })
        );
    }

    #[test]
    fn non_object_root_falls_back() {
        assert_eq!(
            normalize_root("t", json!({"type": "string"})),
            json!({"type": "object", "properties": {}})
        );
        assert_eq!(
            normalize_root(
                "t",
                json!({"oneOf": [{"type": "string"}, {"type": "number"}]})
            ),
            json!({"type": "object", "properties": {}})
        );
    }

    #[test]
    fn nested_combinators_merge_until_depth_limit() {
        let leaf = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let shallow = json!({"oneOf": [json!({"anyOf": [json!({"allOf": [leaf.clone()]})]})]});
        assert_eq!(
            normalize_root("t", shallow),
            json!({"type": "object", "properties": {"x": {"type": "string"}}})
        );

        let mut deep = leaf;
        for _ in 0..(MAX_DEPTH + 2) {
            deep = json!({"oneOf": [deep]});
        }
        assert_eq!(
            normalize_root("t", deep),
            json!({"type": "object", "properties": {}})
        );
    }
}
