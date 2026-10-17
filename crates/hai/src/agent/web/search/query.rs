//! Google-style search directives → structured constraints, then per-dimension lenient filtering of
//! the returned results.

use super::SearchResult;

/// One constraint a directive can name. The identifier is the directive's own spelling: `InUrl` is
/// `inurl`, the word a query writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
enum Dim {
    Site,
    InUrl,
    InTitle,
    Filetype,
    Text,
}

impl Dim {
    fn label(self) -> &'static str {
        self.into()
    }
}

/// One directive: a `prefix:value` pair, or a quoted phrase becoming a text constraint. A leading
/// `-` excludes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    dim: Dim,
    value: String,
    exclude: bool,
}

pub(super) struct Query {
    rules: Vec<Rule>,
}

impl Query {
    pub(super) fn parse(raw: &str) -> Self {
        let mut rules = Vec::new();
        for token in tokenize(raw) {
            let (exclude, body) = match token.text.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_owned()),
                _ => (false, token.text.clone()),
            };
            let (dim, value) = match body.split_once(':') {
                Some((prefix, value)) if !value.is_empty() => {
                    // `text:` stays a plain word: quoting is how a text constraint is written, and a
                    // bare word stays fuzzy for the provider to interpret.
                    match prefix.to_ascii_lowercase().parse::<Dim>() {
                        Ok(dim) if dim != Dim::Text => (Some(dim), value),
                        _ => (None, body.as_str()),
                    }
                }
                _ => (None, body.as_str()),
            };
            match dim {
                Some(dim) => rules.push(Rule {
                    dim,
                    value: value.to_owned(),
                    exclude,
                }),
                // Only quoted phrases take part in local text matching: bare words stay fuzzy and
                // are left to the provider.
                None if token.quoted => rules.push(Rule {
                    dim: Dim::Text,
                    value: value.to_owned(),
                    exclude,
                }),
                None => {}
            }
        }
        Self { rules }
    }

    /// With constraints, ask for extra results up front: filtering afterwards may leave fewer than
    /// `limit`.
    pub(super) fn widen(&self, limit: u32) -> u32 {
        if self.rules.is_empty() {
            limit
        } else {
            limit.saturating_mul(2)
        }
    }
}

#[derive(Debug)]
struct Token {
    text: String,
    quoted: bool,
}

fn tokenize(raw: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in raw.chars() {
        if ch == '"' {
            if quoted {
                tokens.push(Token {
                    text: std::mem::take(&mut current),
                    quoted: true,
                });
                quoted = false;
            } else {
                if !current.is_empty() {
                    tokens.push(Token {
                        text: std::mem::take(&mut current),
                        quoted: false,
                    });
                }
                quoted = true;
            }
        } else if ch.is_whitespace() && !quoted {
            if !current.is_empty() {
                tokens.push(Token {
                    text: std::mem::take(&mut current),
                    quoted: false,
                });
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(Token {
            text: current,
            quoted,
        });
    }
    tokens
}

/// Dimension by dimension (within a dimension includes are a union and an exclude vetoes; across
/// dimensions the constraints are ANDed). A dimension that would empty the result set is dropped
/// instead of applied, and reported as a note.
pub(super) fn apply(query: &Query, results: Vec<SearchResult>) -> (Vec<SearchResult>, Vec<String>) {
    let mut current = results;
    let mut notes = Vec::new();
    for dim in [
        Dim::Site,
        Dim::InUrl,
        Dim::InTitle,
        Dim::Filetype,
        Dim::Text,
    ] {
        let rules: Vec<&Rule> = query.rules.iter().filter(|rule| rule.dim == dim).collect();
        if rules.is_empty() {
            continue;
        }
        let includes: Vec<&Rule> = rules.iter().copied().filter(|rule| !rule.exclude).collect();
        let excludes: Vec<&Rule> = rules.iter().copied().filter(|rule| rule.exclude).collect();
        let kept: Vec<SearchResult> = current
            .iter()
            .filter(|result| {
                (includes.is_empty()
                    || includes
                        .iter()
                        .any(|rule| matches_dim(dim, &rule.value, result)))
                    && !excludes
                        .iter()
                        .any(|rule| matches_dim(dim, &rule.value, result))
            })
            .cloned()
            .collect();
        if kept.is_empty() {
            notes.push(format!(
                "{} matched nothing; constraint relaxed",
                rules
                    .iter()
                    .map(|rule| describe(rule))
                    .collect::<Vec<String>>()
                    .join(" ")
            ));
        } else {
            current = kept;
        }
    }
    (current, notes)
}

fn describe(rule: &Rule) -> String {
    let sign = if rule.exclude { "-" } else { "" };
    format!("{sign}{}:{}", rule.dim.label(), rule.value)
}

fn matches_dim(dim: Dim, value: &str, result: &SearchResult) -> bool {
    match dim {
        Dim::Site => host_path_matches(&result.url, value),
        Dim::InUrl => result.url.to_lowercase().contains(&value.to_lowercase()),
        Dim::InTitle => result.title.to_lowercase().contains(&value.to_lowercase()),
        Dim::Filetype => extension_matches(&result.url, value),
        Dim::Text => {
            let haystack = format!(
                "{} {}",
                result.title,
                result.snippet.as_deref().unwrap_or_default()
            )
            .to_lowercase();
            haystack.contains(&value.to_lowercase())
        }
    }
}

/// `site:docs.rs` matches the host and its subdomains; `site:docs.rs/tokio` additionally requires
/// the path prefix.
fn host_path_matches(url: &str, value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    let path = parsed.path().to_lowercase();
    let value = value.trim_end_matches('/').to_lowercase();
    let (host_part, path_part) = match value.split_once('/') {
        Some((host_part, path_part)) => (host_part, Some(path_part)),
        None => (value.as_str(), None),
    };
    let host_ok = host == host_part || host.ends_with(&format!(".{host_part}"));
    match path_part {
        Some(path_part) => host_ok && path.starts_with(&format!("/{path_part}")),
        None => host_ok,
    }
}

fn extension_matches(url: &str, value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let extension = std::path::Path::new(parsed.path())
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    extension.eq_ignore_ascii_case(value.trim_start_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(title: &str, url: &str, snippet: Option<&str>) -> SearchResult {
        SearchResult {
            title: title.to_owned(),
            url: url.to_owned(),
            snippet: snippet.map(str::to_owned),
            age: None,
        }
    }

    fn sample() -> Vec<SearchResult> {
        vec![
            result(
                "Tokio tutorial",
                "https://docs.rs/tokio/latest/tokio/",
                Some("getting started"),
            ),
            result(
                "Tokio blog",
                "https://tokio.rs/blog/2026",
                Some("release notes"),
            ),
            result(
                "Reddit thread",
                "https://reddit.com/r/rust/x",
                Some("tokio discussion"),
            ),
        ]
    }

    #[test]
    fn parse_keeps_prefixed_and_quoted_constraints() {
        let query = Query::parse(
            r#"site:docs.rs tokio -site:reddit.com "exact phrase" -blog filetype:PDF"#,
        );
        assert_eq!(
            query.rules,
            vec![
                Rule {
                    dim: Dim::Site,
                    value: "docs.rs".to_owned(),
                    exclude: false
                },
                Rule {
                    dim: Dim::Site,
                    value: "reddit.com".to_owned(),
                    exclude: true
                },
                Rule {
                    dim: Dim::Text,
                    value: "exact phrase".to_owned(),
                    exclude: false
                },
                Rule {
                    dim: Dim::Filetype,
                    value: "PDF".to_owned(),
                    exclude: false
                },
            ]
        );
        assert_eq!(Query::parse("").rules, Vec::new());
        assert_eq!(Query::parse("plain words only").rules, Vec::new());
    }

    /// A directive is the enum's own spelling, and `text:` is not one of them: quoting is how a text
    /// constraint is written, so an unquoted `text:` word stays fuzzy.
    #[test]
    fn only_the_declared_directives_are_read() {
        let query = Query::parse("inurl:a intitle:b text:c filetype:d");
        assert_eq!(
            query.rules,
            vec![
                Rule {
                    dim: Dim::InUrl,
                    value: "a".to_owned(),
                    exclude: false
                },
                Rule {
                    dim: Dim::InTitle,
                    value: "b".to_owned(),
                    exclude: false
                },
                Rule {
                    dim: Dim::Filetype,
                    value: "d".to_owned(),
                    exclude: false
                },
            ]
        );
    }

    #[test]
    fn widen_only_inflates_when_constrained() {
        assert_eq!(Query::parse("plain").widen(10), 10);
        assert_eq!(Query::parse("site:docs.rs").widen(10), 20);
    }

    #[test]
    fn site_constraint_filters_to_host_and_subdomains() {
        let query = Query::parse("site:docs.rs");
        let (kept, notes) = apply(&query, sample());
        assert_eq!(kept.len(), 1);
        assert!(kept[0].url.contains("docs.rs"));
        assert!(notes.is_empty());

        let query = Query::parse("site:tokio.rs");
        let (kept, _) = apply(&query, sample());
        assert_eq!(kept.len(), 1, "docs.rs is not a subdomain of tokio.rs");
        assert!(kept[0].url.starts_with("https://tokio.rs/"));

        let query = Query::parse("site:reddit.com/r/rust");
        let (kept, _) = apply(&query, sample());
        assert_eq!(
            kept.len(),
            1,
            "a site: value with a path requires a path match"
        );
    }

    #[test]
    fn two_includes_in_one_dimension_union() {
        let query = Query::parse("site:docs.rs site:tokio.rs");
        let (kept, notes) = apply(&query, sample());
        assert_eq!(
            kept.len(),
            2,
            "values within one dimension are a union, not an intersection"
        );
        assert!(notes.is_empty());

        let query = Query::parse("site:docs.rs -site:reddit.com");
        let (kept, _) = apply(&query, sample());
        assert_eq!(kept.len(), 1, "an include with an exclude still vetoes");
        assert!(kept[0].url.contains("docs.rs"));
    }

    #[test]
    fn exclusion_and_phrase_dimensions_apply() {
        let query = Query::parse("-site:reddit.com");
        let (kept, _) = apply(&query, sample());
        assert_eq!(kept.len(), 2);

        let query = Query::parse("intitle:blog");
        let (kept, _) = apply(&query, sample());
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title, "Tokio blog");

        let query = Query::parse(r#""release notes""#);
        let (kept, _) = apply(&query, sample());
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].url, "https://tokio.rs/blog/2026");

        let query = Query::parse("filetype:pdf");
        let (kept, notes) = apply(&query, sample());
        assert_eq!(
            kept.len(),
            3,
            "the filetype dimension empties the result set, so it is relaxed"
        );
        assert_eq!(
            notes,
            vec!["filetype:pdf matched nothing; constraint relaxed".to_owned()]
        );
    }

    #[test]
    fn empty_dimension_is_relaxed_with_a_note() {
        let query = Query::parse("site:docs.rs intitle:blog");
        let (kept, notes) = apply(&query, sample());
        assert_eq!(
            kept.len(),
            1,
            "the site dimension matched; the intitle dimension was relaxed"
        );
        assert_eq!(
            notes,
            vec!["intitle:blog matched nothing; constraint relaxed".to_owned()]
        );
    }

    #[test]
    fn no_constraints_pass_results_through() {
        let query = Query::parse("tokio");
        let (kept, notes) = apply(&query, sample());
        assert_eq!(kept.len(), 3);
        assert!(notes.is_empty());
    }
}
