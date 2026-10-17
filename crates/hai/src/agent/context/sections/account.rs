use super::fmt::display_name;
use crate::{agentcore::render::elements::Node, domain::model::Account};

pub fn account_element(account: &Account) -> Node {
    let meta = account.platform_meta();

    let mut b = Node::tag("account").attr("id", account.id.0);

    if let Some(m) = &meta {
        if let Some(u) = m.username() {
            b = b.attr("username", format!("@{}", u));
        }
        b = b.attr("name", m.full_name());
    } else {
        b = b.attr("name", display_name(Some(account), account.id));
    }

    if let Some(iid) = account.identity_id {
        b = b.attr("identity_id", iid.0);
    }

    b
}
