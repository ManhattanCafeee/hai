use std::{collections::HashMap, hash::Hash};

/// Reciprocal rank fusion constant: each source contributes `1/(K + rank)`.
const RRF_K: f64 = 60.0;

/// Fuses ranked lists by reciprocal rank, best-first, truncated to `limit`. Rank alone decides a
/// contribution, so the fused lists never have to agree on a score scale.
pub(crate) fn reciprocal_rank_fusion<K: Eq + Hash, T>(
    lists: Vec<Vec<T>>,
    key: impl Fn(&T) -> K,
    limit: usize,
) -> Vec<(T, f64)> {
    let mut acc: HashMap<K, (T, f64)> = HashMap::new();
    for list in lists {
        for (rank, item) in list.into_iter().enumerate() {
            let contribution = 1.0 / (RRF_K + (rank + 1) as f64);
            acc.entry(key(&item))
                .and_modify(|(_, score)| *score += contribution)
                .or_insert((item, contribution));
        }
    }

    let mut hits: Vec<(T, f64)> = acc.into_values().collect();
    hits.sort_by(|a, b| b.1.total_cmp(&a.1));
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_items_outrank_single_source_items() {
        let fused = reciprocal_rank_fusion(vec![vec!["a", "b"], vec!["b", "c"]], |item| *item, 10);

        let items: Vec<&str> = fused.iter().map(|(item, _)| *item).collect();
        assert_eq!(items, ["b", "a", "c"]);
        assert!(fused[0].1 > fused[1].1, "two contributions beat one");
    }

    #[test]
    fn one_list_keeps_its_own_order() {
        let fused = reciprocal_rank_fusion(vec![vec!["a", "b"]], |item| *item, 10);

        let items: Vec<&str> = fused.iter().map(|(item, _)| *item).collect();
        assert_eq!(items, ["a", "b"]);
    }

    #[test]
    fn limit_truncates_the_fused_order() {
        let fused = reciprocal_rank_fusion(vec![vec!["a", "b", "c"]], |item| *item, 2);

        let items: Vec<&str> = fused.iter().map(|(item, _)| *item).collect();
        assert_eq!(items, ["a", "b"]);
    }
}
