use rand::seq::SliceRandom;
use rand::Rng;

use crate::library::CatalogItem;
use crate::state::Ordering;

/// Picks the next item to print. `random` chooses uniformly at random,
/// avoiding an immediate repeat of `last_item_id` when more than one item
/// exists. `sequential` advances past `last_item_id` in list order,
/// wrapping around, or starts at the first item if there is no last item
/// (fresh state, or the last-printed item was since removed).
pub fn choose_item<'a>(items: &'a [CatalogItem], ordering: Ordering, last_item_id: &Option<String>, rng: &mut impl Rng) -> Option<&'a CatalogItem> {
    if items.is_empty() {
        return None;
    }
    match ordering {
        Ordering::Random => {
            let candidates: Vec<&CatalogItem> = items.iter().filter(|item| Some(&item.id) != last_item_id.as_ref()).collect();
            let pool = if candidates.is_empty() { items.iter().collect::<Vec<_>>() } else { candidates };
            pool.choose(rng).copied()
        }
        Ordering::Sequential => {
            let last_index = last_item_id.as_ref().and_then(|id| items.iter().position(|item| &item.id == id));
            let next_index = last_index.map(|index| (index + 1) % items.len()).unwrap_or(0);
            items.get(next_index)
        }
    }
}

pub fn random_delay_seconds(min_minutes: f64, max_minutes: f64, rng: &mut impl Rng) -> f64 {
    if max_minutes <= min_minutes {
        return min_minutes * 60.0;
    }
    rng.gen_range(min_minutes..max_minutes) * 60.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn item(id: &str) -> CatalogItem {
        CatalogItem { id: id.to_string(), original_filename: String::new(), added_at: 0.0, print_count: 0, last_printed_at: None }
    }

    #[test]
    fn sequential_advances_past_the_last_item_and_wraps() {
        let items = vec![item("a"), item("b"), item("c")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert_eq!(choose_item(&items, Ordering::Sequential, &Some("a".into()), &mut rng).unwrap().id, "b");
        assert_eq!(choose_item(&items, Ordering::Sequential, &Some("c".into()), &mut rng).unwrap().id, "a");
        assert_eq!(choose_item(&items, Ordering::Sequential, &None, &mut rng).unwrap().id, "a");
    }

    #[test]
    fn sequential_starts_over_if_the_last_printed_item_was_removed() {
        let items = vec![item("a"), item("b")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert_eq!(choose_item(&items, Ordering::Sequential, &Some("no-longer-exists".into()), &mut rng).unwrap().id, "a");
    }

    #[test]
    fn random_never_returns_none_for_a_non_empty_list() {
        let items = vec![item("a")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert_eq!(choose_item(&items, Ordering::Random, &None, &mut rng).unwrap().id, "a");
    }

    #[test]
    fn choose_item_returns_none_for_an_empty_library() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert!(choose_item(&[], Ordering::Random, &None, &mut rng).is_none());
    }
}
