//! The lexical leg's vocabulary: the terms a keyword search matches with, and the patterns it binds.

/// Build an `ILIKE` pattern: escape `\`, `%`, and `_`, then wrap the keyword in `%`.
fn like_pattern(keyword: &str) -> String {
    let mut out = String::with_capacity(keyword.len() + 2);
    out.push('%');
    for ch in keyword.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

/// Terms one lexical leg matches with; more patterns widen the `ILIKE ANY` without sharpening it.
pub(crate) const MAX_LEXICAL_TERMS: usize = 16;

pub(crate) fn patterns(keywords: &[String]) -> Vec<String> {
    keywords
        .iter()
        .map(|keyword| like_pattern(keyword))
        .collect()
}

/// The `ILIKE ANY` predicate a lexical leg filters on: every column is matched against the pattern
/// array bound at `$idx`. Two or more columns become one parenthesised `OR` chain; a single column
/// stays bare, so it can sit directly after `AND`.
pub(crate) fn match_predicate(columns: &[&str], idx: usize) -> String {
    match columns {
        [column] => format!("{column} ILIKE ANY(${idx})"),
        columns => format!(
            "({})",
            columns
                .iter()
                .map(|column| format!("{column} ILIKE ANY(${idx})"))
                .collect::<Vec<_>>()
                .join(" OR ")
        ),
    }
}

/// The `ORDER BY` arm ranking a lexical leg by how many patterns a row matches, most first.
pub(crate) fn match_rank(columns: &[&str], idx: usize) -> String {
    let matches = columns
        .iter()
        .map(|column| format!("{column} ILIKE p"))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!("(SELECT count(*) FROM unnest(${idx}::text[]) p WHERE {matches}) DESC")
}

/// The terms a keyword leg searches for. A latin run is kept whole when it is at least three bytes;
/// a run carrying CJK is expanded into its adjacent character pairs, because Postgres cannot
/// tokenize CJK and a whole-sentence pattern would match nothing. Order-preserving, deduped, capped.
pub(crate) fn lexical_terms(text: &str, max: usize) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for run in text.split(|c: char| !c.is_alphanumeric()) {
        if run.is_empty() {
            continue;
        }
        if run.chars().any(is_cjk) {
            let chars: Vec<char> = run.chars().collect();
            for pair in chars.windows(2) {
                push_term(&mut terms, pair.iter().collect(), max);
            }
        } else if run.len() >= 3 {
            push_term(&mut terms, run.to_owned(), max);
        }
    }
    terms
}

fn push_term(terms: &mut Vec<String>, term: String, max: usize) {
    if terms.len() < max && !terms.contains(&term) {
        terms.push(term);
    }
}

/// The scripts Postgres cannot tokenize: kana, CJK extensions, CJK unified, compatibility
/// ideographs and Hangul syllables.
fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7AF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern("plain"), "%plain%");
        assert_eq!(like_pattern("100%"), "%100\\%%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
        assert_eq!(like_pattern("c:\\x"), "%c:\\\\x%");
    }

    #[test]
    fn patterns_escape_every_keyword() {
        assert_eq!(
            patterns(&["100%".to_owned(), "a_b".to_owned()]),
            ["%100\\%%", "%a\\_b%"]
        );
        assert!(patterns(&[]).is_empty());
    }

    #[test]
    fn a_single_column_predicate_stays_bare() {
        assert_eq!(match_predicate(&["content"], 5), "content ILIKE ANY($5)");
    }

    #[test]
    fn several_columns_become_one_parenthesised_or_chain() {
        assert_eq!(
            match_predicate(&["t.title", "t.summary"], 4),
            "(t.title ILIKE ANY($4) OR t.summary ILIKE ANY($4))"
        );
    }

    #[test]
    fn the_match_rank_counts_matches_and_orders_most_first() {
        assert_eq!(
            match_rank(&["c.content"], 2),
            "(SELECT count(*) FROM unnest($2::text[]) p WHERE c.content ILIKE p) DESC"
        );
        assert_eq!(
            match_rank(&["t.title", "t.summary"], 4),
            "(SELECT count(*) FROM unnest($4::text[]) p WHERE t.title ILIKE p \
             OR t.summary ILIKE p) DESC"
        );
    }

    #[test]
    fn lexical_terms_expand_cjk_into_adjacent_pairs() {
        assert_eq!(
            lexical_terms("上次那个报错怎么办", MAX_LEXICAL_TERMS),
            [
                "上次", "次那", "那个", "个报", "报错", "错怎", "怎么", "么办"
            ]
        );
    }

    #[test]
    fn lexical_terms_keep_latin_runs_and_drop_short_ones() {
        assert_eq!(
            lexical_terms("timeout in IO handler bug", MAX_LEXICAL_TERMS),
            ["timeout", "handler", "bug"]
        );
    }

    #[test]
    fn lexical_terms_dedupe_and_cap() {
        assert_eq!(lexical_terms("报错 报错", MAX_LEXICAL_TERMS), ["报错"]);
        assert_eq!(lexical_terms("上次那个", 2), ["上次", "次那"]);
        assert!(lexical_terms("。，、", MAX_LEXICAL_TERMS).is_empty());
    }
}
