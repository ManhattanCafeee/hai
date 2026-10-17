pub mod elements;
pub use elements::{AttrValue, Node};

/// Render one element as the `<context>` XML the model reads.
pub fn render_xml(node: &Node) -> String {
    let mut out = String::new();
    render_node(node, &mut out, 0);
    out
}

/// The item-style JSON a tool result carries: `_tag`, the attributes, and `children`. `_tag` and
/// `children` are reserved keys — an attribute of either name never overrides them.
pub fn render_json(node: &Node) -> serde_json::Value {
    match node {
        Node::Text(t) => serde_json::Value::String(t.clone()),
        Node::Elem {
            tag,
            attrs,
            children,
        } => {
            let mut map = serde_json::Map::new();
            for (key, value) in attrs {
                map.insert(
                    key.clone(),
                    serde_json::to_value(value).expect("AttrValue serializes as a bare scalar"),
                );
            }
            map.insert("_tag".to_string(), serde_json::Value::String(tag.clone()));
            if !children.is_empty() {
                map.insert(
                    "children".to_string(),
                    serde_json::Value::Array(children.iter().map(render_json).collect()),
                );
            }
            serde_json::Value::Object(map)
        }
    }
}

fn render_node(node: &Node, out: &mut String, depth: usize) {
    match node {
        Node::Elem {
            tag,
            attrs,
            children,
        } => render_elem(tag, attrs, children, out, depth),
        Node::Text(t) => {
            out.push_str(&"  ".repeat(depth));
            escape_text(t, out);
            out.push('\n');
        }
    }
}

fn render_elem(
    tag: &str,
    attrs: &indexmap::IndexMap<String, AttrValue>,
    children: &[Node],
    out: &mut String,
    depth: usize,
) {
    out.push_str(&"  ".repeat(depth));
    out.push('<');
    out.push_str(tag);
    for (key, value) in attrs {
        match value {
            // Bool attributes are marker attributes: present means true, so no value is emitted,
            // and `false` omits the attribute entirely.
            AttrValue::Bool(true) => {
                out.push(' ');
                out.push_str(key);
            }
            AttrValue::Bool(false) => {}
            _ => {
                out.push(' ');
                out.push_str(key);
                out.push_str("=\"");
                escape_attr(&value.to_string(), out);
                out.push('"');
            }
        }
    }

    if children.is_empty() {
        out.push_str("/>\n");
        return;
    }

    out.push_str(">\n");
    for child in children {
        render_node(child, out, depth + 1);
    }
    out.push_str(&"  ".repeat(depth));
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_attrs_render_bare() {
        let node = Node::tag("msg")
            .attr("id", 1i64)
            .attr("own", true)
            .attr("urgent", AttrValue::Bool(true));
        assert_eq!(render_xml(&node), "<msg id=\"1\" own urgent/>\n");
    }

    #[test]
    fn text_and_attr_escaped() {
        let node = Node::tag("t").child(Node::Text("a & b".into()));
        assert_eq!(render_xml(&node), "<t>\n  a &amp; b\n</t>\n");
    }

    #[test]
    fn an_attribute_never_overrides_the_reserved_keys() {
        let node = Node::tag("msg").attr("_tag", "not-the-tag");
        assert_eq!(render_json(&node)["_tag"], serde_json::json!("msg"));
    }
}
