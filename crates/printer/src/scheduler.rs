use rand::seq::SliceRandom;
use rand::Rng;

use crate::library::CatalogItem;
use crate::state::Ordering;

/// Picks the next item to print. `sequential` advances past `last_item_id`
/// in list order, wrapping around, or starts at the first item if there is
/// no last item (fresh state, or the last-printed item was since removed).
///
/// `random` prints a full shuffled pass over the library before any item
/// repeats, then reshuffles and starts another pass -- `shuffle_queue`
/// holds the remaining not-yet-printed ids for the current pass (the
/// caller persists it across ticks). Ids for items removed since the pass
/// began are dropped as they're encountered; an item added mid-pass isn't
/// retroactively inserted, but is naturally included in the next reshuffle
/// once the current pass empties.
pub fn choose_item<'a>(
    items: &'a [CatalogItem],
    ordering: Ordering,
    last_item_id: &Option<String>,
    shuffle_queue: &mut Vec<String>,
    rng: &mut impl Rng,
) -> Option<&'a CatalogItem> {
    if items.is_empty() {
        shuffle_queue.clear();
        return None;
    }
    match ordering {
        Ordering::Random => {
            shuffle_queue.retain(|id| items.iter().any(|item| &item.id == id));
            if shuffle_queue.is_empty() {
                let mut ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
                ids.shuffle(rng);
                *shuffle_queue = ids;
            }
            let next_id = shuffle_queue.remove(0);
            items.iter().find(|item| item.id == next_id)
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
        let mut queue = Vec::new();
        assert_eq!(choose_item(&items, Ordering::Sequential, &Some("a".into()), &mut queue, &mut rng).unwrap().id, "b");
        assert_eq!(choose_item(&items, Ordering::Sequential, &Some("c".into()), &mut queue, &mut rng).unwrap().id, "a");
        assert_eq!(choose_item(&items, Ordering::Sequential, &None, &mut queue, &mut rng).unwrap().id, "a");
    }

    #[test]
    fn sequential_starts_over_if_the_last_printed_item_was_removed() {
        let items = vec![item("a"), item("b")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let mut queue = Vec::new();
        assert_eq!(choose_item(&items, Ordering::Sequential, &Some("no-longer-exists".into()), &mut queue, &mut rng).unwrap().id, "a");
    }

    #[test]
    fn random_never_returns_none_for_a_non_empty_list() {
        let items = vec![item("a")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let mut queue = Vec::new();
        assert_eq!(choose_item(&items, Ordering::Random, &None, &mut queue, &mut rng).unwrap().id, "a");
    }

    #[test]
    fn choose_item_returns_none_for_an_empty_library() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let mut queue = Vec::new();
        assert!(choose_item(&[], Ordering::Random, &None, &mut queue, &mut rng).is_none());
    }

    #[test]
    fn random_visits_every_item_exactly_once_before_any_repeat() {
        let items = vec![item("a"), item("b"), item("c"), item("d")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let mut queue = Vec::new();
        let mut picked = Vec::new();
        for _ in 0..items.len() {
            picked.push(choose_item(&items, Ordering::Random, &None, &mut queue, &mut rng).unwrap().id.clone());
        }
        picked.sort();
        assert_eq!(picked, vec!["a", "b", "c", "d"]);
        assert!(queue.is_empty(), "queue should be fully drained after one full pass");
    }

    #[test]
    fn random_reshuffles_into_a_new_full_pass_once_exhausted() {
        let items = vec![item("a"), item("b"), item("c")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut queue = Vec::new();
        let mut first_pass = Vec::new();
        let mut second_pass = Vec::new();
        for _ in 0..items.len() {
            first_pass.push(choose_item(&items, Ordering::Random, &None, &mut queue, &mut rng).unwrap().id.clone());
        }
        for _ in 0..items.len() {
            second_pass.push(choose_item(&items, Ordering::Random, &None, &mut queue, &mut rng).unwrap().id.clone());
        }
        first_pass.sort();
        second_pass.sort();
        assert_eq!(first_pass, vec!["a", "b", "c"]);
        assert_eq!(second_pass, vec!["a", "b", "c"]);
    }

    #[test]
    fn random_drops_queued_ids_for_items_removed_mid_pass() {
        let items = vec![item("a"), item("b")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        // Simulate a pass that started when "c" still existed but was since removed.
        let mut queue = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        let picked = choose_item(&items, Ordering::Random, &None, &mut queue, &mut rng).unwrap();
        assert!(picked.id == "a" || picked.id == "b");
        assert!(!queue.contains(&"c".to_string()));
    }
}
