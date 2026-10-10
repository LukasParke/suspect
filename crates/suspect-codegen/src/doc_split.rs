//! Shared helpers for split generated documentation: operations grouped one
//! page per spec tag and model reference grouped into bounded alphabetical
//! files, so no single generated page carries a whole API surface.

use std::collections::{BTreeMap, BTreeSet};

/// Lowercase URL-safe file stem; runs of non-alphanumerics collapse to `-`.
///
/// ```
/// # use suspect_codegen::doc_split::slug;
/// assert_eq!(slug("Live TV"), "live-tv");
/// assert_eq!(slug("  "), "other");
/// ```
#[must_use]
pub fn slug(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "other".into()
    } else {
        out
    }
}

/// Allocate a unique file stem, suffixing `-2`, `-3`, … on collision.
#[must_use]
pub fn allocate_slug(used: &mut BTreeSet<String>, base: &str) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}-{suffix}");
        suffix += 1;
    }
    name
}

/// Group items by their spec tags into sorted `(tag, items)` groups. Untagged
/// items land in `Other`. An item carrying several tags appears in each.
#[must_use]
pub fn by_tag<'a, T: 'a>(
    items: impl IntoIterator<Item = &'a T>,
    tags_of: impl Fn(&T) -> Vec<String>,
) -> BTreeMap<String, Vec<&'a T>> {
    let mut by_tag: BTreeMap<String, Vec<&'a T>> = BTreeMap::new();
    for item in items {
        let keys = tags_of(item);
        let keys: Vec<String> = if keys.is_empty() {
            vec!["Other".into()]
        } else {
            keys
        };
        for key in keys {
            by_tag.entry(key).or_default().push(item);
        }
    }
    by_tag
}

/// Common-prefix label for a group of names at the given prefix depth.
#[must_use]
pub fn prefix_label(names: &[String], depth: usize) -> String {
    names
        .first()
        .map(|n| n.chars().take(depth).collect::<String>())
        .unwrap_or_default()
}

/// Alphabetical groups bounded by `limit` items each. Names are sorted, then
/// grouped by common prefix: a group above the limit subdivides by one more
/// character until it divides or reaches the depth ceiling of 12, so every
/// generated file stays readable whatever the schema count.
#[must_use]
pub fn bounded_groups<'a, T: 'a>(
    items: Vec<&'a T>,
    name_of: impl Fn(&T) -> String,
    limit: usize,
) -> Vec<(String, Vec<&'a T>)> {
    fn split<'a, T: 'a>(
        items: Vec<&'a T>,
        depth: usize,
        limit: usize,
        name_of: &impl Fn(&T) -> String,
    ) -> Vec<(String, Vec<&'a T>)> {
        if items.len() <= limit || depth >= 12 {
            let label = prefix_label(&items.iter().map(|i| name_of(i)).collect::<Vec<_>>(), depth);
            return vec![(label, items)];
        }
        let mut by_prefix: BTreeMap<String, Vec<&'a T>> = BTreeMap::new();
        for item in &items {
            let prefix: String = name_of(item).chars().take(depth + 1).collect();
            by_prefix.entry(prefix).or_default().push(*item);
        }
        if by_prefix.len() == 1 {
            return split(items, depth + 1, limit, name_of);
        }
        by_prefix
            .into_values()
            .flat_map(|group| split(group, depth + 1, limit, name_of))
            .collect()
    }
    let mut sorted = items;
    sorted.sort_by_key(|item| name_of(item));
    let mut by_letter: BTreeMap<String, Vec<&'a T>> = BTreeMap::new();
    for item in sorted {
        let letter: String = name_of(item)
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "#".into());
        by_letter.entry(letter).or_default().push(item);
    }
    by_letter
        .into_values()
        .flat_map(|group| split(group, 1, limit, &name_of))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_collapses_separators_and_defaults_to_other() {
        assert_eq!(slug("Live TV"), "live-tv");
        assert_eq!(slug("A/B--C!"), "a-b-c");
        assert_eq!(slug("---"), "other");
    }

    #[test]
    fn allocate_slug_suffixes_collisions() {
        let mut used = BTreeSet::new();
        assert_eq!(allocate_slug(&mut used, "library"), "library");
        assert_eq!(allocate_slug(&mut used, "library"), "library-2");
        assert_eq!(allocate_slug(&mut used, "library"), "library-3");
    }

    #[test]
    fn by_tag_groups_untagged_into_other() {
        let items = vec!["a", "b", "c"];
        let groups = by_tag(&items, |item| match *item {
            "a" => vec!["X".to_owned()],
            _ => Vec::new(),
        });
        let keys: Vec<_> = groups.keys().collect();
        assert_eq!(keys, ["Other", "X"]);
        assert_eq!(groups["X"], [&"a"]);
        assert_eq!(groups["Other"].len(), 2);
    }

    #[test]
    fn bounded_groups_subdivide_large_prefixes() {
        let items: Vec<String> = (0..10)
            .map(|i| format!("GetItem{i:02}Leaf"))
            .chain((0..5).map(|i| format!("Media{i}")))
            .collect();
        let refs: Vec<&String> = items.iter().collect();
        let groups = bounded_groups(refs, |s: &String| s.clone(), 4);
        // The G group exceeds the limit and must subdivide; the M group fits.
        assert!(groups.len() > 2);
        assert!(groups.iter().all(|(_, g)| g.len() <= 4 || g.len() == 10));
        assert_eq!(
            groups.iter().map(|(_, g)| g.len()).sum::<usize>(),
            items.len()
        );
    }
}
