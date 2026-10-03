use super::search::Row;
use crate::localization::search_key;
use serde::{Deserialize, Serialize};

pub(crate) const MAX_SMART_CRATES: usize = 64;
pub(crate) const MAX_CONDITIONS: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Combine {
    #[default]
    All,
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TextField {
    Title,
    Artist,
    Key,
    Tag,
    Group,
    Note,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TextMatch {
    Contains,
    Equals,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NumberField {
    Bpm,
    Length,
    Rating,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Condition {
    Text {
        field: TextField,
        comparison: TextMatch,
        value: String,
    },
    Number {
        field: NumberField,
        minimum: f64,
        maximum: f64,
    },
    Played {
        value: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rule {
    pub combine: Combine,
    pub conditions: Vec<Condition>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            combine: Combine::All,
            conditions: vec![Condition::Text {
                field: TextField::Key,
                comparison: TextMatch::Equals,
                value: "C".into(),
            }],
        }
    }
}

impl Rule {
    /// Check a saved smart crate rule.
    /// Takes this rule; rejects empty, oversized, unknown or invalid conditions before publication.
    pub fn validate(&self) -> Result<(), String> {
        if self.conditions.is_empty() || self.conditions.len() > MAX_CONDITIONS {
            return Err("A smart crate needs 1–16 conditions".into());
        }
        for condition in &self.conditions {
            match condition {
                Condition::Text { value, .. }
                    if value.trim().is_empty()
                        || value.len() > 256
                        || value.chars().any(char::is_control) =>
                {
                    return Err(
                        "Smart crate text needs 1–256 bytes without control characters".into(),
                    )
                }
                Condition::Number {
                    field,
                    minimum,
                    maximum,
                } => {
                    let limit = match field {
                        NumberField::Bpm => 1000.0,
                        NumberField::Length => 604800.0,
                        NumberField::Rating => 5.0,
                    };
                    if !minimum.is_finite()
                        || !maximum.is_finite()
                        || *minimum < 0.0
                        || minimum > maximum
                        || *maximum > limit
                        || *field == NumberField::Rating
                            && (minimum.fract() != 0.0 || maximum.fract() != 0.0)
                    {
                        return Err("Use an ordered finite range: BPM 0–1000, length 0–604800 seconds, or whole ratings 0–5".into());
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Compile a validated typed rule once on a worker.
    /// Takes this rule; returns normalized predicates or a visible validation error.
    pub fn compile(&self) -> Result<Compiled, String> {
        self.validate()?;
        let mut rule = self.clone();
        for condition in &mut rule.conditions {
            if let Condition::Text { value, .. } = condition {
                *value = search_key(value);
            }
        }
        Ok(Compiled(rule))
    }
}

pub(crate) struct Compiled(Rule);
impl Compiled {
    /// Match current metadata without changing media or transport.
    /// Takes a borrowed library row; unknown numeric metadata never satisfies a range.
    pub fn matches(&self, row: Row<'_>) -> bool {
        let matches = |condition: &Condition| match condition {
            Condition::Text {
                field,
                comparison,
                value,
            } => {
                let matches = |text: &str| {
                    let text = search_key(text);
                    match comparison {
                        TextMatch::Contains => text.contains(value),
                        TextMatch::Equals => text == *value,
                    }
                };
                match field {
                    TextField::Title => matches(row.title),
                    TextField::Artist => matches(row.artist),
                    TextField::Key => matches(row.key),
                    TextField::Tag => row.annotations.tags.iter().any(|tag| matches(tag)),
                    TextField::Group => matches(&row.annotations.group),
                    TextField::Note => matches(&row.annotations.notes),
                }
            }
            Condition::Number {
                field,
                minimum,
                maximum,
            } => {
                let value = match field {
                    NumberField::Bpm => row.bpm.map(f64::from),
                    NumberField::Length => row.seconds,
                    NumberField::Rating => Some(row.annotations.rating as f64),
                };
                value.is_some_and(|value| (*minimum..=*maximum).contains(&value))
            }
            Condition::Played { value } => row.played == *value,
        };
        match self.0.combine {
            Combine::All => self.0.conditions.iter().all(matches),
            Combine::Any => self.0.conditions.iter().any(matches),
        }
    }
}

#[cfg(test)]
mod tests;
