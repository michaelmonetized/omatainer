use super::annotations::Annotations;
use crate::localization::search_key;

pub(crate) struct Indexed {title:String,artist:String,key:String,bpm:Option<f32>,seconds:Option<f64>,annotations:Annotations,sorted_tags:String}
impl Indexed {
    /// Normalize one exact metadata record once on its owning worker.
    /// Takes borrowed effective fields; retains full Unicode search semantics without shortening text.
    pub(crate) fn new(row:Row<'_>)->Self {
        Self {sorted_tags:search_key(&row.annotations.tags.join(", ")),title:search_key(row.title),artist:search_key(row.artist),key:search_key(row.key),bpm:row.bpm,seconds:row.seconds,annotations:Annotations {rating:row.annotations.rating,color:row.annotations.color,group:search_key(&row.annotations.group),notes:search_key(&row.annotations.notes),tags:row.annotations.tags.iter().map(|tag|search_key(tag)).collect()}}
    }
    /// Borrow prepared Unicode fields for a current query or sort.
    /// Takes the live confirmed play state; returns exact normalized metadata without allocating.
    pub(crate) fn row(&self,played:bool)->Row<'_> {Row {title:&self.title,artist:&self.artist,key:&self.key,bpm:self.bpm,seconds:self.seconds,played,annotations:&self.annotations}}
    /// Borrow the same comma-separated tag order used by library sorting.
    /// Takes no arguments; returns full normalized joined tags prepared on the worker.
    pub(crate) fn sorted_tags(&self)->&str {&self.sorted_tags}
    /// Account for retained normalized field capacity.
    /// Takes no arguments; returns record and string/vector storage bytes without shared-owner headers.
    pub(crate) fn bytes(&self)->usize {std::mem::size_of::<Self>()+self.title.capacity()+self.artist.capacity()+self.key.capacity()+self.annotations.group.capacity()+self.annotations.notes.capacity()+self.sorted_tags.capacity()+self.annotations.tags.capacity()*std::mem::size_of::<String>()+self.annotations.tags.iter().map(String::capacity).sum::<usize>()}
}
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
    pub fn matches(&self, row: Row<'_>) -> bool {self.matches_fields(row,false)}
    /// Match a prepared exact metadata record without allocating normalized text.
    /// Takes the worker-owned record and live confirmed play state; returns whether every compiled predicate matches.
    pub(crate) fn matches_indexed(&self,row:&Indexed,played:bool)->bool {self.matches_fields(Row {title:&row.title,artist:&row.artist,key:&row.key,bpm:row.bpm,seconds:row.seconds,played,annotations:&row.annotations},true)}
    fn matches_fields(&self,row:Row<'_>,normalized:bool)->bool {
        self.0.iter().all(|predicate| match predicate {
            Predicate::Text(field, value) => {
                let contains = |text: &str| if normalized {text.contains(value)} else {search_key(text).contains(value)};
                let equals=|text:&str|if normalized {text==value} else {search_key(text)==*value};
                match field {
                    Field::Any => {
                        contains(row.title)
                            || contains(row.artist)
                            || contains(row.key)
                            || contains(&row.annotations.group) || contains(&row.annotations.notes) || row.annotations.tags.iter().any(|tag|contains(tag))
                    }
                    Field::Title => contains(row.title),
                    Field::Artist => contains(row.artist),
                    Field::Key => equals(row.key),
                    Field::Tag => row
                        .annotations
                        .tags
                        .iter()
                        .any(|tag| equals(tag)),
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
