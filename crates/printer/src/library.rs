use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: String,
    pub original_filename: String,
    pub added_at: f64,
    pub print_count: u32,
    pub last_printed_at: Option<f64>,
}

fn sidecar_path(complete_dir: &Path, id: &str) -> std::path::PathBuf {
    complete_dir.join(format!("{id}.json"))
}

fn zpl_path(complete_dir: &Path, id: &str) -> std::path::PathBuf {
    complete_dir.join(format!("{id}.zpl"))
}

/// Lists every catalog item, oldest-added first -- a stable, predictable
/// order for "sequential" scheduling and for the library-management UI.
pub fn list(complete_dir: &Path) -> Vec<CatalogItem> {
    let Ok(entries) = fs::read_dir(complete_dir) else {
        return Vec::new();
    };
    let mut items: Vec<CatalogItem> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str::<CatalogItem>(&text).ok())
        .collect();
    items.sort_by(|a, b| a.added_at.partial_cmp(&b.added_at).unwrap_or(std::cmp::Ordering::Equal));
    items
}

pub fn get(complete_dir: &Path, id: &str) -> Option<CatalogItem> {
    let text = fs::read_to_string(sidecar_path(complete_dir, id)).ok()?;
    serde_json::from_str(&text).ok()
}

/// An id containing a path separator or a `..` component could otherwise
/// escape `complete_dir` when joined into a path -- reject it up front and
/// let the caller treat it exactly like an unknown id (404), rather than
/// giving path-traversal attempts a distinct error shape to probe with.
fn is_valid_id(id: &str) -> bool {
    !id.contains('/') && !id.contains("..")
}

/// Removes both the sidecar and the print-ready ZPL file. Returns the
/// removed item's metadata so the caller (the HTTP API) can echo it back,
/// or `None` if no such item exists (the caller answers 404).
pub fn remove(complete_dir: &Path, id: &str) -> Option<CatalogItem> {
    if !is_valid_id(id) {
        return None;
    }
    let item = get(complete_dir, id)?;
    let _ = fs::remove_file(zpl_path(complete_dir, id));
    let _ = fs::remove_file(sidecar_path(complete_dir, id));
    Some(item)
}

pub fn read_zpl(complete_dir: &Path, id: &str) -> std::io::Result<String> {
    fs::read_to_string(zpl_path(complete_dir, id))
}

pub fn mark_printed(complete_dir: &Path, id: &str, printed_at: f64) -> std::io::Result<()> {
    let Some(mut item) = get(complete_dir, id) else {
        return Ok(()); // Item was removed between selection and printing -- nothing to update.
    };
    item.print_count += 1;
    item.last_printed_at = Some(printed_at);
    fs::write(sidecar_path(complete_dir, id), serde_json::to_vec_pretty(&item).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_item(dir: &Path, id: &str, added_at: f64) {
        let item = CatalogItem {
            id: id.to_string(),
            original_filename: format!("{id}.png"),
            added_at,
            print_count: 0,
            last_printed_at: None,
        };
        fs::write(dir.join(format!("{id}.json")), serde_json::to_vec(&item).unwrap()).unwrap();
        fs::write(dir.join(format!("{id}.zpl")), b"^XA\n^XZ\n").unwrap();
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("library-test-{}-{}", std::process::id(), rand_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    #[test]
    fn list_returns_items_oldest_first() {
        let dir = temp_dir();
        write_item(&dir, "second", 200.0);
        write_item(&dir, "first", 100.0);
        let items = list(&dir);
        assert_eq!(items.iter().map(|i| i.id.clone()).collect::<Vec<_>>(), vec!["first", "second"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_deletes_both_files_and_returns_the_item() {
        let dir = temp_dir();
        write_item(&dir, "one", 1.0);
        let removed = remove(&dir, "one").unwrap();
        assert_eq!(removed.id, "one");
        assert!(!dir.join("one.json").exists());
        assert!(!dir.join("one.zpl").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_returns_none_for_an_unknown_id() {
        let dir = temp_dir();
        assert!(remove(&dir, "missing").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_rejects_ids_containing_a_path_separator_or_dot_dot() {
        let dir = temp_dir();
        assert!(remove(&dir, "../../etc/passwd").is_none());
        assert!(remove(&dir, "/etc/passwd").is_none());
        assert!(remove(&dir, "foo/../bar").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mark_printed_increments_the_count_and_sets_last_printed_at() {
        let dir = temp_dir();
        write_item(&dir, "one", 1.0);
        mark_printed(&dir, "one", 500.0).unwrap();
        let item = get(&dir, "one").unwrap();
        assert_eq!(item.print_count, 1);
        assert_eq!(item.last_printed_at, Some(500.0));
        let _ = fs::remove_dir_all(&dir);
    }
}
