//! Pure and deterministic: the same content and config always produce the same chunk sequence.

mod scan;

use self::scan::{heading_level, scan_blocks};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkCfg {
    /// Target chunk length, in Unicode scalar values.
    pub size: usize,
    /// Overlap between adjacent chunks, in Unicode scalar values.
    pub overlap: usize,
    /// Unit size that triggers a split, and the budget a sub-split works within, in Unicode
    /// scalar values.
    pub max: usize,
}

impl Default for ChunkCfg {
    fn default() -> Self {
        Self {
            size: 512,
            overlap: 51,
            max: 1536,
        }
    }
}

/// Split a document into chunk texts; each chunk carries its heading-path prefix and the overlap
/// tail copied from its predecessor.
///
/// Empty or blank-only input yields no chunks. Content is never truncated or dropped, so a unit
/// that cannot be split below `max` (a single oversized line, table row, or code line) stays
/// oversized; callers own any hard limit they need to enforce.
pub fn chunk(content: &str, cfg: &ChunkCfg) -> Vec<String> {
    let blocks = scan_blocks(content);
    if blocks.is_empty() {
        return Vec::new();
    }
    let sections = build_sections(blocks);
    let mut out: Vec<Chunk> = Vec::new();
    for sec in sections {
        for piece in chunk_units(&sec.heading_path, &sec.units, cfg) {
            out.push(piece);
        }
    }
    apply_overlap(&mut out, cfg.overlap);
    out.into_iter().map(|c| c.into_text()).collect()
}

/// Determines how an oversized unit is split and whether that unit may take part in overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitKind {
    /// Prose paragraph: splittable at sentence boundaries, participates in overlap.
    Paragraph,
    /// Block quote: splittable at sentence boundaries, participates in overlap.
    Quote,
    /// Fenced code block: protected — never cut except at its own blank lines.
    Code,
    /// Table: protected — split only between rows, every sub-chunk repeating the header.
    Table,
    /// List group: protected — split between items, an oversized item falling back to
    /// sentence boundaries.
    List,
}

#[derive(Debug, Clone)]
struct Unit {
    kind: UnitKind,
    text: String,
}

#[derive(Debug, Clone)]
struct Chunk {
    /// Heading path prefix; empty at the document top level.
    heading_path: Vec<String>,
    body: String,
    // Overlap must not cross into or out of a protected unit (code, table, list), so both chunk
    // edges keep their unit kind.
    first_kind: UnitKind,
    last_kind: UnitKind,
}

impl Chunk {
    fn into_text(self) -> String {
        let mut s = String::new();
        for (i, h) in self.heading_path.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            s.push_str(h);
        }
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&self.body);
        s
    }
}

struct Section {
    heading_path: Vec<String>,
    units: Vec<Unit>,
}

fn build_sections(blocks: Vec<Unit>) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut pending = false;

    for unit in blocks {
        // A heading arrives as a single-line Paragraph and must satisfy the same validity rule
        // as `classify_line`.
        if unit.kind == UnitKind::Paragraph && !unit.text.contains('\n') {
            let trimmed = unit.text.trim_start();
            if let Some(level) = heading_level(trimmed) {
                while path.len() >= level as usize {
                    path.pop();
                }
                path.push(unit.text.trim().to_string());
                pending = true;
                continue;
            }
        }
        if pending || sections.is_empty() {
            sections.push(Section {
                heading_path: path.clone(),
                units: Vec::new(),
            });
            pending = false;
        }
        sections
            .last_mut()
            .expect("section pushed above")
            .units
            .push(unit);
    }
    sections
}

fn chunk_units(heading_path: &[String], units: &[Unit], cfg: &ChunkCfg) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::new();
    let mut cur: Vec<Unit> = Vec::new();
    let mut cur_len = 0usize;

    let flush = |cur: &mut Vec<Unit>, out: &mut Vec<Chunk>, path: &[String]| {
        if cur.is_empty() {
            return;
        }
        let first_kind = cur.first().map(|u| u.kind).unwrap_or(UnitKind::Paragraph);
        let last_kind = cur.last().map(|u| u.kind).unwrap_or(UnitKind::Paragraph);
        let body = cur
            .iter()
            .map(|u| u.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        out.push(Chunk {
            heading_path: path.to_vec(),
            body,
            first_kind,
            last_kind,
        });
        cur.clear();
    };

    for unit in units {
        let unit_len = unit.text.chars().count();
        if unit_len > cfg.max {
            flush(&mut cur, &mut out, heading_path);
            cur_len = 0;
            for piece in split_unit(unit, cfg) {
                out.push(Chunk {
                    heading_path: heading_path.to_vec(),
                    body: piece,
                    first_kind: unit.kind,
                    last_kind: unit.kind,
                });
            }
            continue;
        }
        // `size` drives aggregation; `max` only decides when a single unit has to be split.
        if cur_len + unit_len + 1 > cfg.size && !cur.is_empty() {
            flush(&mut cur, &mut out, heading_path);
            cur_len = 0;
        }
        cur_len += unit_len + 1;
        cur.push(unit.clone());
    }
    flush(&mut cur, &mut out, heading_path);
    out
}

fn split_unit(unit: &Unit, cfg: &ChunkCfg) -> Vec<String> {
    match unit.kind {
        UnitKind::Code => split_code(&unit.text, cfg),
        UnitKind::Table => split_table(&unit.text, cfg),
        UnitKind::List => split_list(&unit.text, cfg),
        UnitKind::Paragraph | UnitKind::Quote => split_text(&unit.text, cfg),
    }
}

/// Split a code block at blank lines; every sub-chunk re-emits the fence, language tag included.
fn split_code(text: &str, cfg: &ChunkCfg) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let (open, rest) = match lines.first() {
        Some(first) => (first.to_string(), &lines[1..]),
        None => return Vec::new(),
    };
    let close = lines
        .last()
        .filter(|l| {
            let t = l.trim_end().trim_start();
            let mut it = t.chars();
            match it.next() {
                Some('`') | Some('~') => it.all(|c| c == '`' || c == '~'),
                _ => false,
            }
        })
        .map(|l| l.to_string())
        .unwrap_or_default();
    let inner: Vec<&str> = if close.is_empty() {
        rest.to_vec()
    } else {
        rest[..rest.len().saturating_sub(1)].to_vec()
    };

    let mut pieces: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for &l in &inner {
        if l.trim().is_empty() && !cur.is_empty() {
            pieces.push(std::mem::take(&mut cur));
        } else {
            cur.push(l);
        }
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    if pieces.is_empty() {
        pieces.push(inner);
    }

    let mut out: Vec<String> = Vec::new();
    for piece in pieces {
        let mut block = String::new();
        block.push_str(&open);
        block.push('\n');
        block.push_str(&piece.join("\n"));
        if !close.is_empty() {
            block.push('\n');
            block.push_str(&close);
        }
        if block.chars().count() <= cfg.max {
            out.push(block);
        } else {
            for sub in split_by_line_count(&piece, cfg) {
                let mut b = String::new();
                b.push_str(&open);
                b.push('\n');
                b.push_str(&sub.join("\n"));
                if !close.is_empty() {
                    b.push('\n');
                    b.push_str(&close);
                }
                out.push(b);
            }
        }
    }
    out
}

/// Heuristic line-wise split. A single line longer than the budget is not cut, so a piece can
/// still exceed `max`.
fn split_by_line_count<'a>(lines: &[&'a str], cfg: &ChunkCfg) -> Vec<Vec<&'a str>> {
    // Pieces target half of `max`: each one gets its fences and its join re-added on top.
    let target_chars = cfg.max.saturating_sub(8) / 2;
    let mut out: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_len = 0usize;
    for &l in lines {
        let lc = l.chars().count();
        if cur_len + lc > target_chars && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
        cur_len += lc;
        cur.push(l);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Split a table by character budget, repeating the header and separator rows in every sub-chunk.
/// A table holding only those two rows stays whole.
fn split_table(text: &str, cfg: &ChunkCfg) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 2 {
        return vec![text.to_string()];
    }
    let head = &lines[..2];
    let data = &lines[2..];
    let head_len = head.iter().map(|l| l.chars().count()).sum::<usize>() + 2;
    let budget = cfg.max.saturating_sub(head_len + 2);
    let mut out: Vec<String> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_len = 0usize;
    for row in data {
        let rc = row.chars().count() + 1;
        if cur_len + rc > budget && !cur.is_empty() {
            out.push(build_table_block(head, &cur));
            cur.clear();
            cur_len = 0;
        }
        cur_len += rc;
        cur.push(row);
    }
    if !cur.is_empty() {
        out.push(build_table_block(head, &cur));
    }
    out
}

fn build_table_block(head: &[&str], rows: &[&str]) -> String {
    let mut b = String::new();
    b.push_str(&head.join("\n"));
    b.push('\n');
    b.push_str(&rows.join("\n"));
    b
}

/// Split a list group at item boundaries; an item that is still oversized falls back to sentence
/// boundaries with its marker kept.
fn split_list(text: &str, cfg: &ChunkCfg) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_len = 0usize;
    for &l in &lines {
        let lc = l.chars().count();
        if cur_len + lc > cfg.max && !cur.is_empty() {
            out.push(cur.join("\n"));
            cur.clear();
            cur_len = 0;
        }
        cur_len += lc + 1;
        cur.push(l);
    }
    if !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    let mut final_out: Vec<String> = Vec::new();
    for piece in out {
        if piece.chars().count() <= cfg.max {
            final_out.push(piece);
        } else {
            for sub in split_text(&piece, cfg) {
                final_out.push(sub);
            }
        }
    }
    final_out
}

/// Greedy sentence-boundary split up to `max`; when no boundary is in reach it hard-cuts at a
/// character boundary, so no Unicode scalar is split.
fn split_text(text: &str, cfg: &ChunkCfg) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let len = rest.chars().count();
        if len <= cfg.max {
            out.push(rest.to_string());
            break;
        }
        let window: Vec<(usize, char)> = rest.char_indices().take(cfg.max).collect();
        let mut cut = None;
        for (idx, ch) in window.iter().rev() {
            if is_sentence_boundary(*ch) {
                cut = Some(idx + ch.len_utf8());
                break;
            }
        }
        let cut = match cut {
            Some(cut) => cut,
            // An empty window means no budget at all: emitting the remainder is the only progress
            // left.
            None if window.is_empty() => {
                out.push(rest.to_string());
                break;
            }
            None => window.last().map(|(i, c)| i + c.len_utf8()).unwrap_or(0),
        };
        out.push(rest[..cut].to_string());
        rest = &rest[cut..];
    }
    out
}

fn is_sentence_boundary(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';' | '\n')
}

/// Copy up to `k` characters of the previous chunk's tail into the head of the next one.
///
/// Both edges must be unprotected units (code, table, and list are structural boundaries), and the
/// two chunks must share a heading path: a heading is a semantic boundary, so overlapping across it
/// would pull the previous section's text in.
fn apply_overlap(chunks: &mut [Chunk], k: usize) {
    if k == 0 {
        return;
    }
    for i in 1..chunks.len() {
        if chunks[i].heading_path != chunks[i - 1].heading_path {
            continue;
        }
        let (prev_body, prev_kind) = {
            let prev = &chunks[i - 1];
            (prev.body.clone(), prev.last_kind)
        };
        if !matches!(prev_kind, UnitKind::Paragraph | UnitKind::Quote)
            || !matches!(chunks[i].first_kind, UnitKind::Paragraph | UnitKind::Quote)
        {
            continue;
        }
        let tail = tail_up_to(&prev_body, k);
        if tail.is_empty() {
            continue;
        }
        chunks[i].body.insert_str(0, &tail);
    }
}

/// Up to `k` characters from the end of `text`, trimmed back to the last sentence boundary inside
/// that window so no sentence is cut.
fn tail_up_to(text: &str, k: usize) -> String {
    let total = text.chars().count();
    if total <= k {
        return text.to_string();
    }
    let skip = total - k;
    let start = text.char_indices().nth(skip).map(|(i, _)| i).unwrap_or(0);
    let tail = &text[start..];
    let mut cut_at = None;
    for (i, c) in tail.char_indices() {
        if is_sentence_boundary(c) {
            cut_at = Some(i + c.len_utf8());
        }
    }
    match cut_at {
        Some(end) => tail[..end].to_string(),
        None => tail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ChunkCfg {
        ChunkCfg {
            size: 512,
            overlap: 51,
            max: 1536,
        }
    }

    #[test]
    fn empty_doc_yields_no_chunks() {
        assert!(chunk("", &cfg()).is_empty());
        assert!(chunk("   \n\n  ", &cfg()).is_empty());
    }

    #[test]
    fn short_doc_single_chunk() {
        let doc = "# 标题\n\n一段内容。";
        let out = chunk(doc, &cfg());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], "# 标题\n一段内容。");
    }

    #[test]
    fn zero_max_cannot_split() {
        let out = chunk(
            "aaaa",
            &ChunkCfg {
                size: 8,
                overlap: 2,
                max: 0,
            },
        );
        assert_eq!(out, vec!["aaaa".to_owned()], "no budget means no split");
    }

    #[test]
    fn heading_split_adds_path_prefix() {
        let doc = "# A\n\naaaa。\n\n# B\n\nbbbb。";
        let out = chunk(doc, &cfg());
        assert_eq!(out.len(), 2);
        assert!(out[0].starts_with("# A\n"));
        assert!(out[1].starts_with("# B\n"));
    }

    #[test]
    fn nested_heading_path() {
        let doc = "# A\n\n## B\n\ncontent。";
        let out = chunk(doc, &cfg());
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("# A\n## B\ncontent"));
    }

    #[test]
    fn code_fence_preserved() {
        let doc = "# 代码\n\n```rust\nfn main() {}\n```";
        let out = chunk(doc, &cfg());
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("```rust\nfn main() {}\n```"));
    }

    #[test]
    fn oversized_code_split_keeps_fences() {
        let cfg = ChunkCfg {
            size: 30,
            overlap: 0,
            max: 30,
        };
        let code = format!(
            "```rust\n{}\n\n{}\n\n{}",
            "x".repeat(20),
            "y".repeat(20),
            "z".repeat(20)
        );
        let doc = format!("# T\n\n{code}\n```");
        let out = chunk(&doc, &cfg);
        assert!(out.len() >= 2, "got {} chunks: {out:?}", out.len());
        for c in &out {
            assert!(
                c.starts_with("# T\n```rust\n"),
                "sub-chunk keeps the fence and the language tag: {c:?}"
            );
            assert!(
                c.ends_with("```"),
                "sub-chunk ends with the closing fence: {c:?}"
            );
        }
    }

    #[test]
    fn oversized_table_repeats_header() {
        let cfg = ChunkCfg {
            size: 20,
            overlap: 0,
            max: 40,
        };
        let table = "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n| 5 | 6 |";
        let doc = format!("# T\n\n{table}");
        let out = chunk(&doc, &cfg);
        for c in &out {
            assert!(
                c.contains("| a | b |\n|---|---|"),
                "every sub-chunk repeats the table header: {c}"
            );
        }
    }

    #[test]
    fn text_split_at_sentence_boundary() {
        let cfg = ChunkCfg {
            size: 10,
            overlap: 0,
            max: 12,
        };
        let doc = format!("# T\n\n{}", "一二三四五六七八九十。第二句话。第三句话。");
        let out = chunk(&doc, &cfg);

        assert!(out.len() > 1, "{out:?}");
        // Every chunk but the last ends on sentence-final punctuation; the last may end mid-sentence.
        for chunk in &out[..out.len() - 1] {
            let body = chunk.strip_prefix("# T\n").expect("the title prefix");
            assert!(
                body.ends_with('。'),
                "a middle chunk was cut mid-sentence: {body:?}"
            );
        }
    }

    #[test]
    fn emoji_not_split_mid_scalar() {
        let cfg = ChunkCfg {
            size: 3,
            overlap: 0,
            max: 5,
        };
        let doc = "🦀🦀🦀🦀🦀";
        let out = chunk(doc, &cfg);
        for c in &out {
            assert!(
                c.chars().all(|ch| ch == '🦀'),
                "a chunk must not split a Unicode scalar: {c:?}"
            );
        }
    }

    #[test]
    fn overlap_applied_between_text_chunks() {
        let cfg = ChunkCfg {
            size: 20,
            overlap: 10,
            max: 40,
        };
        let body = format!("{}。{}。", "x".repeat(30), "y".repeat(30));
        let doc = format!("# T\n\n{body}");
        let out = chunk(&doc, &cfg);
        assert!(out.len() > 1);
        // The second chunk carries the tail of the first: after the sentence-boundary trim it
        // still holds "x" content.
        assert!(
            out[1].contains('x'),
            "overlap should carry the tail of the previous chunk: {out:?}"
        );
    }

    #[test]
    fn overlap_skipped_across_code_boundary() {
        let cfg = ChunkCfg {
            size: 30,
            overlap: 20,
            max: 60,
        };
        let doc = format!("# T\n\n{}\n\n```\ncode block\n```", "a".repeat(80));
        let out = chunk(&doc, &cfg);
        assert!(out.len() > 1);
        // Code sub-chunks take no overlap, so the code chunk never starts with "a".
        let code_chunk = out.iter().find(|c| c.contains("```")).unwrap();
        assert!(
            !code_chunk.strip_prefix("# T\n").unwrap().starts_with('a'),
            "overlap must not cross into the code block: {code_chunk:?}"
        );
    }

    #[test]
    fn blank_lines_separate_paragraphs_not_units() {
        let doc = "# T\n\n第一段。\n\n第二段。";
        let out = chunk(doc, &cfg());
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("第一段。"));
        assert!(out[0].contains("第二段。"));
    }

    #[test]
    fn list_group_not_split_across_items() {
        let doc = "# T\n\n- 甲\n- 乙\n- 丙";
        let out = chunk(doc, &cfg());
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("- 甲\n- 乙\n- 丙"));
    }

    #[test]
    fn fenced_block_with_language_tag_closes() {
        // Regression: an opening fence with a language tag must still be closed by "```", or it
        // swallows the rest of the document.
        let doc = "# A\n\n```rust\nfn main() {}\n```\n\n# B\n\n正文乙。";
        let out = chunk(doc, &cfg());
        assert_eq!(
            out.len(),
            2,
            "a new section should open normally after the code block: {out:?}"
        );
        assert!(out[0].contains("```rust\nfn main() {}\n```"));
        assert!(out[1].starts_with("# B\n正文乙"));
    }

    #[test]
    fn english_prose_not_misclassified_as_list() {
        // Regression: prose containing ". " is not an ordered list.
        let doc = "# T\n\nHello. World.\n\ne.g. example text.";
        let out = chunk(doc, &cfg());
        assert_eq!(
            out.len(),
            1,
            "prose must not be split into a list group: {out:?}"
        );
        assert!(out[0].contains("Hello. World."));
        assert!(out[0].contains("e.g. example text."));
    }

    #[test]
    fn ordered_list_still_detected() {
        let doc = "# T\n\n1. 第一步\n2. 第二步\n\n3) 三";
        let out = chunk(doc, &cfg());
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("1. 第一步\n2. 第二步"));
    }

    #[test]
    fn size_target_drives_aggregation() {
        // Regression: aggregation follows `size` (the target), not `max` (the ceiling).
        let cfg = ChunkCfg {
            size: 150,
            overlap: 0,
            max: 1000,
        };
        // Four 55-character paragraphs: size=150 fits two per chunk (111 <= 150) for 2 chunks;
        // aggregating by max would give 1.
        let body: String = (0..4)
            .map(|i| format!("段落{i}：{}。", "字".repeat(50)))
            .collect::<Vec<_>>()
            .join("\n\n");
        let doc = format!("# T\n\n{body}");
        let out = chunk(&doc, &cfg);
        assert_eq!(
            out.len(),
            2,
            "size=150 should aggregate into 2 chunks: {out:?}"
        );
    }

    #[test]
    fn tiny_table_without_data_rows_not_lost() {
        // Regression: an oversized table with only a header and separator row is not lost.
        let cfg = ChunkCfg {
            size: 5,
            overlap: 0,
            max: 10,
        };
        let table = "| a |\n|---|";
        let doc = format!("# T\n\n{table}");
        let out = chunk(&doc, &cfg);
        assert_eq!(
            out.len(),
            1,
            "a table without data rows must not be dropped: {out:?}"
        );
        assert!(out[0].contains("| a |\n|---|"));
    }

    #[test]
    fn hash_pound_without_space_is_text_not_heading() {
        // Regression: "#foo" is not a heading; classification and section building agree on this.
        let doc = "#foo 不是标题。\n\n正文。";
        let out = chunk(doc, &cfg());
        assert_eq!(out.len(), 1);
        assert!(
            out[0].starts_with("#foo 不是标题。"),
            "it should stay text, not become a heading: {out:?}"
        );
    }

    #[test]
    fn fence_with_three_plus_markers() {
        let doc = "# A\n\n````\ncode\n````\n\n# B\n\n乙。";
        let out = chunk(doc, &cfg());
        assert_eq!(out.len(), 2);
        assert!(out[0].contains("````\ncode\n````"));
        assert!(out[1].starts_with("# B"));
    }

    #[test]
    fn indented_fence_closes_without_swallowing() {
        // Regression: an indented fence ("  ```bash" / "  ```") must close normally, so the list
        // and the text after it are not swallowed; overlap never crosses sections.
        let doc = "# A\n\n  ```bash\n  pacman -S x\n  ```\n\n- AMD\n\n  开源驱动说明。\n\n# B\n\n正文乙。";
        let out = chunk(doc, &cfg());
        assert_eq!(
            out.len(),
            2,
            "each of the two sections should become one chunk: {out:?}"
        );
        let a = &out[0];
        // The closing fence sits after pacman and before AMD, which is outside the block.
        assert!(
            a.contains("  ```bash\n  pacman -S x\n  ```\n- AMD"),
            "the closing fence should come before AMD: {a:?}"
        );
        assert!(a.contains("开源驱动说明"));
        // No overlap across sections: the second chunk holds only the "# B" content.
        assert_eq!(
            out[1], "# B\n正文乙。",
            "overlap must not leak across the heading boundary: {out:?}"
        );
    }
}
