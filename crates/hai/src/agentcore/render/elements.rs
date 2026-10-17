use std::fmt::{Display, Formatter};

use indexmap::IndexMap;
use serde::Serialize;

#[derive(Debug, Clone)]
pub enum AttrValue {
    String(String),
    Int(i64),
    Bool(bool),
}

impl Serialize for AttrValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            AttrValue::String(s) => serializer.serialize_str(s),
            AttrValue::Int(i) => serializer.serialize_i64(*i),
            AttrValue::Bool(b) => serializer.serialize_bool(*b),
        }
    }
}

impl Display for AttrValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AttrValue::String(s) => write!(f, "{}", s),
            AttrValue::Int(i) => write!(f, "{}", i),
            AttrValue::Bool(b) => write!(f, "{}", b),
        }
    }
}

impl From<String> for AttrValue {
    fn from(s: String) -> Self {
        AttrValue::String(s)
    }
}

impl From<&String> for AttrValue {
    fn from(s: &String) -> Self {
        AttrValue::String(s.clone())
    }
}

impl From<&str> for AttrValue {
    fn from(s: &str) -> Self {
        AttrValue::String(s.to_string())
    }
}

impl From<i64> for AttrValue {
    fn from(i: i64) -> Self {
        AttrValue::Int(i)
    }
}

impl From<i32> for AttrValue {
    fn from(i: i32) -> Self {
        AttrValue::Int(i as i64)
    }
}

impl From<bool> for AttrValue {
    fn from(b: bool) -> Self {
        AttrValue::Bool(b)
    }
}

impl From<uuid::Uuid> for AttrValue {
    fn from(u: uuid::Uuid) -> Self {
        AttrValue::String(u.to_string())
    }
}

impl From<jiff::Timestamp> for AttrValue {
    fn from(t: jiff::Timestamp) -> Self {
        AttrValue::String(t.to_string())
    }
}

#[derive(Debug, Clone)]
pub enum Node {
    Elem {
        tag: String,
        attrs: IndexMap<String, AttrValue>,
        children: Vec<Node>,
    },
    Text(String),
}

impl Node {
    pub fn tag(name: impl Into<String>) -> Self {
        Node::Elem {
            tag: name.into(),
            attrs: IndexMap::new(),
            children: Vec::new(),
        }
    }

    pub fn text(content: impl Into<String>) -> Self {
        Node::Text(content.into())
    }

    pub fn attr(mut self, key: impl Into<String>, value: impl Into<AttrValue>) -> Self {
        if let Node::Elem { ref mut attrs, .. } = self {
            attrs.insert(key.into(), value.into());
        }
        self
    }

    pub fn child(mut self, child: impl Into<Node>) -> Self {
        if let Node::Elem {
            ref mut children, ..
        } = self
        {
            children.push(child.into());
        }
        self
    }

    pub fn children(self, items: Vec<Node>) -> Self {
        match self {
            Node::Elem {
                tag,
                attrs,
                mut children,
            } => {
                children.extend(items);
                Node::Elem {
                    tag,
                    attrs,
                    children,
                }
            }
            Node::Text(_) => self,
        }
    }

    /// Empty means no children and no attributes: an attribute-only element such as `<chat …/>`
    /// still carries content and must not be dropped.
    pub fn is_empty(&self) -> bool {
        match self {
            Node::Elem {
                attrs, children, ..
            } => children.is_empty() && attrs.is_empty(),
            Node::Text(_) => false,
        }
    }
}

impl From<String> for Node {
    fn from(s: String) -> Self {
        Node::Text(s)
    }
}

impl From<&str> for Node {
    fn from(s: &str) -> Self {
        Node::Text(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_only_element_is_not_empty() {
        assert!(Node::tag("situation").is_empty());
        assert!(!Node::tag("chat").attr("id", 2).is_empty());
        assert!(!Node::tag("topic").child(Node::text("summary")).is_empty());
        assert!(!Node::text("").is_empty());
    }
}
