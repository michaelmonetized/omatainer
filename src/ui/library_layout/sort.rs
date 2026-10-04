//! Cached Unicode prefixes bound retained sort-key memory for large crates.
use super::*;
use crate::preferences::library_layout::{Column, Sort};
use std::{borrow::Cow, cmp::Ordering};

enum Key {
    Text(String),
    Number(Option<f64>),
    Time(Option<SystemTime>),
    Color(Option<[u8; 3]>),
}
/// Resolve the text represented by one sortable column.
/// Takes row metadata, annotations and a text column; returns borrowed text or a bounded joined tag list.
fn text<'a>(
    item: &'a LibItem,
    fields: &'a crate::library::annotations::Annotations,
    column: Column,
) -> Cow<'a, str> {
    match column {
        Column::Title => Cow::Borrowed(&item.title),
        Column::Artist => Cow::Borrowed(&item.artist),
        Column::Key => Cow::Borrowed(&item.key),
        Column::Group => Cow::Borrowed(&fields.group),
        Column::Tags => Cow::Owned(fields.tags.join(", ")),
        Column::Notes => Cow::Borrowed(&fields.notes),
        _ => Cow::Borrowed(""),
    }
}
/// Cache only a bounded normalized prefix for each row.
/// Takes a sort column and exact row values; returns a key retaining at most 128 text bytes.
fn key(
    item: &LibItem,
    fields: &crate::library::annotations::Annotations,
    played: Option<SystemTime>,
    column: Column,
) -> Key {
    match column {
        Column::Bpm => Key::Number(
            item.bpm
                .value()
                .filter(|value| value.is_finite())
                .map(f64::from),
        ),
        Column::Length => Key::Number(item.length.filter(|value| value.is_finite())),
        Column::Rating => Key::Number(Some(f64::from(fields.rating))),
        Column::Played => Key::Time(played),
        Column::Color => Key::Color(fields.color),
        _ => {
            let mut normalized = crate::localization::search_key(&text(item, fields, column));
            let mut boundary = normalized.len().min(128);
            while !normalized.is_char_boundary(boundary) {
                boundary -= 1;
            }
            normalized.truncate(boundary);
            Key::Text(normalized)
        }
    }
}
/// Order present values while keeping unavailable metadata last.
/// Takes two optional values and the chosen direction; returns their deterministic comparison.
fn optional<T>(
    a: &Option<T>,
    b: &Option<T>,
    descending: bool,
    cmp: impl FnOnce(&T, &T) -> Ordering,
) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            let result = cmp(a, b);
            if descending {
                result.reverse()
            } else {
                result
            }
        }
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
    }
}
/// Sort one filtered publication with stable ties and exact Unicode comparisons.
/// Takes indices, immutable rows/catalog/history and two sort columns; preserves manual order when no primary sort is selected.
pub(in crate::ui) fn order(
    indices: &mut [usize],
    library: &[LibItem],
    catalog: &crate::library::Catalog,
    history: &play_history::History,
    sorts: [Option<Sort>; 2],
) {
    if sorts[0].is_none() {
        return;
    }
    let empty = crate::library::annotations::Annotations::default();
    let fields = |index: usize| {
        catalog
            .track(&library[index].source)
            .map_or(&empty, |track| &track.annotations)
    };
    let mut rows: Vec<_> = indices
        .iter()
        .map(|&index| {
            let item = &library[index];
            (
                index,
                sorts.map(|sort| {
                    sort.map(|sort| {
                        key(
                            item,
                            fields(index),
                            history.get(item).or(item.last_play),
                            sort.column,
                        )
                    })
                }),
            )
        })
        .collect();
    rows.sort_by(|(a, keys_a), (b, keys_b)| {
        for slot in 0..2 {
            let Some(sort) = sorts[slot] else {
                continue;
            };
            let result = match (
                keys_a[slot].as_ref().unwrap(),
                keys_b[slot].as_ref().unwrap(),
            ) {
                (Key::Text(a_key), Key::Text(b_key)) => {
                    let result = a_key.cmp(b_key);
                    let result = if result == Ordering::Equal {
                        let a_text = text(&library[*a], fields(*a), sort.column);
                        let b_text = text(&library[*b], fields(*b), sort.column);
                        if a_text == b_text {
                            Ordering::Equal
                        } else {
                            crate::localization::search_key(&a_text)
                                .cmp(&crate::localization::search_key(&b_text))
                        }
                    } else {
                        result
                    };
                    if sort.descending {
                        result.reverse()
                    } else {
                        result
                    }
                }
                (Key::Number(a), Key::Number(b)) => {
                    optional(a, b, sort.descending, |a, b| a.total_cmp(b))
                }
                (Key::Time(a), Key::Time(b)) => optional(a, b, sort.descending, Ord::cmp),
                (Key::Color(a), Key::Color(b)) => optional(a, b, sort.descending, Ord::cmp),
                _ => unreachable!("keys share a column"),
            };
            if result != Ordering::Equal {
                return result;
            }
        }
        Ordering::Equal
    });
    for (destination, (index, _)) in indices.iter_mut().zip(rows) {
        *destination = index;
    }
}
