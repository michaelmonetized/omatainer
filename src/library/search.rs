use super::annotations::Annotations;
use crate::localization::search_key;

pub(crate) struct Row<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub key: &'a str,
    pub bpm: Option<f32>,
    pub seconds: Option<f64>,
    pub played: bool,
    pub annotations: &'a Annotations,
}
#[derive(Clone, Copy)]
enum Field {
    Any,
    Title,
    Artist,
    Key,
    Tag,
    Group,
    Note,
}
enum Predicate {
    Text(Field, String),
    Bpm(f64, f64),
    Seconds(f64, f64),
    Rating(f64, f64),
    Played(bool),
    Color([u8; 3]),
}
#[derive(Default)]
pub(crate) struct Query(Vec<Predicate>);
impl Query {
    /// Compile a bounded field-aware library search.
    /// Takes text with optional quoted values; returns predicates that combine with AND or a visible syntax error.
    pub fn parse(input: &str) -> Result<Self, String> {
        if input.len() > 4096 {
            return Err("Library search allows at most 4096 bytes".into());
        }
        let mut terms = Vec::new();
        let mut value = String::new();
        let mut quoted = false;
        let mut escaped = false;
        for character in input.chars() {
            if escaped {
                value.push(character);
                escaped = false;
            } else if character == '\\' && quoted {
                escaped = true;
            } else if character == '"' {
                quoted = !quoted;
            } else if character.is_whitespace() && !quoted {
                if !value.is_empty() {
                    terms.push(std::mem::take(&mut value));
                }
            } else {
                value.push(character);
            }
        }
        if quoted || escaped {
            return Err("Close the quoted search value".into());
        }
        if !value.is_empty() {
            terms.push(value);
        }
        if terms.len() > 64 {
            return Err("Library search allows at most 64 terms".into());
        }
        let mut predicates = Vec::with_capacity(terms.len());
        for term in terms {
            let normalized = search_key(&term);
            let number = ["bpm", "length", "rating"].into_iter().find_map(|field| {
                normalized
                    .strip_prefix(field)
                    .filter(|suffix| suffix.starts_with([':', '>', '<', '=', '!']))
                    .map(|suffix| (field, suffix))
            });
            if let Some((field, suffix)) = number {
                let (min, max) = range(field, suffix)?;
                predicates.push(match field {
                    "bpm" => Predicate::Bpm(min, max),
                    "length" => Predicate::Seconds(min, max),
                    _ => Predicate::Rating(min, max),
                });
                continue;
            }
            let predicate = if let Some((field, value)) = normalized.split_once(':') {
                if value.is_empty() {
                    return Err(format!("Search field {field} needs a value"));
                }
                match field {
                    "title" => Predicate::Text(Field::Title,value.into()),
                    "artist" => Predicate::Text(Field::Artist,value.into()),
                    "key" => Predicate::Text(Field::Key,value.into()),
                    "tag" => Predicate::Text(Field::Tag,value.into()),
                    "group" => Predicate::Text(Field::Group,value.into()),
                    "note" => Predicate::Text(Field::Note,value.into()),
                    "played" => Predicate::Played(match value { "yes"|"true"|"played" => true, "no"|"false"|"unplayed" => false, _ => return Err("Use played:yes or played:no".into()) }),
                    "color" => {
                        let hex = value.strip_prefix('#').ok_or("Use color:#RRGGBB")?;
                        if hex.len()!=6 || !hex.bytes().all(|byte|byte.is_ascii_hexdigit()) { return Err("Use color:#RRGGBB".into()); }
                        Predicate::Color(std::array::from_fn(|index|u8::from_str_radix(&hex[index*2..index*2+2],16).unwrap()))
                    }
                    _ => return Err(format!("Unknown search field {field}; use title, artist, key, tag, group, note, color, bpm, length, rating or played")),
                }
            } else {
                Predicate::Text(Field::Any, normalized)
            };
            predicates.push(predicate);
        }
        Ok(Self(predicates))
    }
    /// Identify searches that depend on confirmed playback.
    /// Takes no arguments; returns whether a played predicate needs history invalidation.
    pub fn uses_play_history(&self) -> bool { self.0.iter().any(|predicate| matches!(predicate, Predicate::Played(_))) }
    /// Match a borrowed current library row.
    /// Takes effective metadata and stable annotations; returns true only when every predicate matches.
    pub fn matches(&self, row: Row<'_>) -> bool {
        self.0.iter().all(|predicate| match predicate {
            Predicate::Text(field, value) => {
                let contains = |text: &str| search_key(text).contains(value);
                match field {
                    Field::Any => {
                        contains(row.title)
                            || contains(row.artist)
                            || contains(row.key)
                            || row.annotations.matches_text(value)
                    }
                    Field::Title => contains(row.title),
                    Field::Artist => contains(row.artist),
                    Field::Key => search_key(row.key) == *value,
                    Field::Tag => row
                        .annotations
                        .tags
                        .iter()
                        .any(|tag| search_key(tag) == *value),
                    Field::Group => contains(&row.annotations.group),
                    Field::Note => contains(&row.annotations.notes),
                }
            }
            Predicate::Bpm(min, max) => row
                .bpm
                .is_some_and(|bpm| (*min..=*max).contains(&(bpm as f64))),
            Predicate::Seconds(min, max) => row
                .seconds
                .is_some_and(|seconds| (*min..=*max).contains(&seconds)),
            Predicate::Rating(min, max) => (*min..=*max).contains(&(row.annotations.rating as f64)),
            Predicate::Played(expected) => row.played == *expected,
            Predicate::Color(expected) => row.annotations.color == Some(*expected),
        })
    }
}
fn range(field: &str, suffix: &str) -> Result<(f64, f64), String> {
    let parse = |value: &str| -> Result<f64, String> {
        let number = if field == "length" && value.contains(':') {
            let parts = value.split(':').collect::<Vec<_>>();
            if parts.len() != 2 {
                return Err("Use length in seconds or minutes:seconds".into());
            }
            let minutes = parts[0]
                .parse::<u32>()
                .map_err(|_| "Invalid length minutes")?;
            let seconds = parts[1]
                .parse::<f64>()
                .map_err(|_| "Invalid length seconds")?;
            if !(0.0..60.0).contains(&seconds) {
                return Err("Length seconds must be below 60".into());
            }
            minutes as f64 * 60.0 + seconds
        } else {
            value
                .parse::<f64>()
                .map_err(|_| format!("Invalid {field} number"))?
        };
        if !number.is_finite()
            || number < 0.0
            || (field == "rating" && (number > 5.0 || number.fract() != 0.0))
        {
            return Err(format!("Invalid {field} number"));
        }
        Ok(number)
    };
    let result = if let Some(value) = suffix.strip_prefix(">=") {
        (parse(value)?, f64::INFINITY)
    } else if let Some(value) = suffix.strip_prefix("<=") {
        (0.0, parse(value)?)
    } else if let Some(value) = suffix
        .strip_prefix(':')
        .or_else(|| suffix.strip_prefix('='))
    {
        if let Some((min, max)) = value.split_once("..") {
            (parse(min)?, parse(max)?)
        } else {
            let number = parse(value)?;
            (number, number)
        }
    } else {
        return Err(format!(
            "Use {field}:min..max, {field}>=value or {field}<=value"
        ));
    };
    if result.0 > result.1 {
        return Err(format!("The {field} range starts above its end"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
