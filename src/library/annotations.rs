//! User performance notes belong to stable tracks, never to embedded audio tags.
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Annotations {
    pub rating: u8,
    pub color: Option<[u8; 3]>,
    pub group: String,
    pub tags: Vec<String>,
    pub notes: String,
}
impl Annotations {
    /// Bound saved user metadata.
    /// Takes this record; rejects ratings outside zero through five and oversized or invalid text.
    pub fn validate(&self) -> Result<(), String> {
        if self.rating > 5
            || !text(&self.group, 256, false)
            || !text(&self.notes, 4096, true)
            || self.tags.len() > 32
            || self
                .tags
                .iter()
                .any(|tag| tag.is_empty() || tag.trim() != tag || !text(tag, 128, false))
        {
            return Err("Annotations allow ratings 0–5, group 256 bytes, notes 4096 bytes and 32 trimmed tags of 128 bytes each".into());
        }
        let mut tags = HashSet::new();
        if self.tags.iter().any(|tag| !tags.insert(tag.to_lowercase())) {
            return Err("Annotation tags must be unique ignoring case".into());
        }
        Ok(())
    }
    /// Omit empty annotations from older-compatible JSON records.
    /// Takes this record; returns whether every user field is unset.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
    /// Format the five virtualized annotation columns.
    /// Takes this record; returns rating, color, group, tags and a one-line notes preview.
    pub fn columns(&self) -> [String; 5] {
        [
            if self.rating == 0 {
                String::new()
            } else {
                format!("{}/5", self.rating)
            },
            self.color
                .map(|c| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
                .unwrap_or_default(),
            self.group.clone(),
            self.tags.join(", "),
            self.notes
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(96)
                .collect(),
        ]
    }
    /// Describe every user field for tooltips and assistive technology.
    /// Takes this record; returns full annotation text, including multiline notes.
    pub fn description(&self) -> String {
        format!(
            "Rating {}/5; color {}; group {}; tags {}; notes {}",
            self.rating,
            self.columns()[1],
            self.group,
            self.tags.join(", "),
            self.notes
        )
    }
    /// Search user text without allocating a combined metadata string.
    /// Takes a lowercase query; matches group, notes or any tag.
    pub fn matches_text(&self, query: &str) -> bool {
        crate::localization::search_key(&self.group).contains(query)
            || crate::localization::search_key(&self.notes).contains(query)
            || self
                .tags
                .iter()
                .any(|tag| crate::localization::search_key(&tag).contains(query))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Patch {
    pub rating: Option<u8>,
    pub color: Option<Option<[u8; 3]>>,
    pub group: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
}
impl Patch {
    /// Apply only fields explicitly selected by the user.
    /// Takes the previous record; returns a validated complete replacement.
    pub fn apply(&self, previous: &Annotations) -> Result<Annotations, String> {
        let next = Annotations {
            rating: self.rating.unwrap_or(previous.rating),
            color: self.color.unwrap_or(previous.color),
            group: self.group.as_ref().unwrap_or(&previous.group).clone(),
            tags: self.tags.as_ref().unwrap_or(&previous.tags).clone(),
            notes: self.notes.as_ref().unwrap_or(&previous.notes).clone(),
        };
        next.validate()?;
        Ok(next)
    }
    /// Detect an empty batch edit.
    /// Takes this patch; returns whether all fields keep their earlier values.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rule {
    pub minimum_rating: u8,
    pub color: Option<[u8; 3]>,
    pub group: String,
    pub tag: String,
    pub notes: String,
}
impl Rule {
    /// Validate one saved annotation rule.
    /// Takes this rule; returns success only for bounded user predicates.
    pub fn validate(&self) -> Result<(), String> {
        if self.minimum_rating > 5
            || !text(&self.group, 256, false)
            || !text(&self.tag, 128, false)
            || !text(&self.notes, 4096, false)
        {
            return Err("Invalid annotation rule: rating must be 0–5 and text must be bounded and control-free".into());
        }
        Ok(())
    }
    /// Evaluate the same rule in search and dynamic crates.
    /// Takes current stable-track annotations; returns true when all configured conditions match.
    pub fn matches(&self, fields: &Annotations) -> bool {
        fields.rating >= self.minimum_rating
            && self.color.is_none_or(|color| fields.color == Some(color))
            && crate::localization::search_key(&fields.group)
                .contains(&crate::localization::search_key(&self.group))
            && crate::localization::search_key(&fields.notes)
                .contains(&crate::localization::search_key(&self.notes))
            && (self.tag.is_empty()
                || fields
                    .tags
                    .iter()
                    .any(|tag| crate::localization::search_key(&tag) == crate::localization::search_key(&self.tag)))
    }
    /// Parse explicit annotation search fields while preserving ordinary search text.
    /// Takes the search string; returns free text plus a rule, or a visible syntax error.
    pub fn search(query: &str) -> Result<(String, Self), String> {
        let mut rule = Self::default();
        let mut ordinary = Vec::new();
        let mut seen = HashSet::new();
        for term in query.split_whitespace() {
            let kind = if term
                .strip_prefix("rating")
                .is_some_and(|suffix| suffix.starts_with(['>', '<', '=', ':', '!']))
            {
                Some("rating")
            } else if term.starts_with("color:") {
                Some("color")
            } else if term.starts_with("group:") {
                Some("group")
            } else if term.starts_with("tag:") {
                Some("tag")
            } else if term.starts_with("note:") {
                Some("note")
            } else {
                None
            };
            if kind.is_some_and(|kind| !seen.insert(kind)) {
                return Err(
                    "Use each annotation search field once; all supplied fields combine".into(),
                );
            }
            if kind == Some("rating") && !term.starts_with("rating>=") {
                return Err("Use rating>=0 through rating>=5".into());
            }
            if let Some(rating) = term.strip_prefix("rating>=") {
                rule.minimum_rating = rating
                    .parse()
                    .map_err(|_| "Use rating>=0 through rating>=5")?;
            } else if let Some(color) = term.strip_prefix("color:") {
                let color = color.strip_prefix('#').ok_or("Use color:#RRGGBB")?;
                if color.len() != 6 {
                    return Err("Use color:#RRGGBB".into());
                }
                let color = u32::from_str_radix(color, 16).map_err(|_| "Use color:#RRGGBB")?;
                rule.color = Some([(color >> 16) as u8, (color >> 8) as u8, color as u8]);
            } else if let Some(group) = term.strip_prefix("group:") {
                if group.is_empty() {
                    return Err("group: needs a word".into());
                }
                rule.group = group.into();
            } else if let Some(tag) = term.strip_prefix("tag:") {
                if tag.is_empty() {
                    return Err("tag: needs a tag".into());
                }
                rule.tag = tag.into();
            } else if let Some(notes) = term.strip_prefix("note:") {
                if notes.is_empty() {
                    return Err("note: needs a word".into());
                }
                rule.notes = notes.into();
            } else {
                ordinary.push(term);
            }
        }
        rule.validate()?;
        Ok((
            if seen.is_empty() {
                query.into()
            } else {
                ordinary.join(" ")
            },
            rule,
        ))
    }
}

/// Validate international user text.
/// Takes a value, byte limit and multiline policy; allows newline/tab only in notes.
fn text(value: &str, maximum: usize, multiline: bool) -> bool {
    value.len() <= maximum
        && !value
            .chars()
            .any(|ch| ch.is_control() && !(multiline && matches!(ch, '\n' | '\t')))
}

impl Catalog {
    /// Edit a frozen batch of stable track identities atomically.
    /// Takes captured IDs and changed fields; refuses duplicates/missing IDs before changing any record.
    pub fn annotate(&mut self, ids: &[TrackId], patch: &Patch) -> Result<bool, String> {
        if ids.is_empty() || ids.len() > MAX_TRACKS || patch.is_empty() {
            return Err(
                "Choose one through 100000 saved tracks and at least one changed field".into(),
            );
        }
        let mut wanted: HashSet<_> = ids.iter().collect();
        if wanted.len() != ids.len() {
            return Err("Duplicate annotation target; nothing changed".into());
        }
        let mut replacements = Vec::with_capacity(ids.len());
        for (index, track) in self.tracks.iter().enumerate() {
            if wanted.remove(&track.id) {
                replacements.push((index, patch.apply(&track.annotations)?));
            }
        }
        if !wanted.is_empty() {
            return Err("An annotation target no longer exists; nothing changed".into());
        }
        let mut changed = false;
        for (index, next) in replacements {
            changed |= self.tracks[index].annotations != next;
            self.tracks[index].annotations = next;
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests;
