use std::{collections::HashMap, fmt::Debug, hash::Hash};

/// Joins ranked rows to their entities by id; a ranked row without an entity (the window between
/// deleting an entity and clearing its vector) is logged and dropped.
pub(super) fn merge_hydrated<K, T>(rows: Vec<(K, f64)>, entities: Vec<(K, T)>) -> Vec<(T, f64)>
where
    K: Eq + Hash + Debug + Copy + std::fmt::Display,
{
    let mut map: HashMap<K, T> = entities.into_iter().collect();
    rows.into_iter()
        .filter_map(|(id, score)| match map.remove(&id) {
            Some(entity) => Some((entity, score)),
            None => {
                tracing::warn!(%id, "ranked row has no entity; skipping");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn merge_keeps_row_order_and_drops_orphans() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let orphan = Uuid::from_u128(3);

        let merged = merge_hydrated(
            vec![(second, 0.2), (orphan, 0.3), (first, 0.1)],
            vec![(first, "a"), (second, "b")],
        );

        assert_eq!(merged, vec![("b", 0.2), ("a", 0.1)]);
    }
}
