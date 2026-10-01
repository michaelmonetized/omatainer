//! Conservative filename hints, never musical-key analysis.
pub(super) const HELP: &str = "File keys are filename hints, not analyzed musical keys. Bare natural keys need a key marker, brackets, or a filename containing only the key.";

fn token_char(c: char) -> bool {
    // Unknown symbols stay attached: unsupported accidentals and combining
    // marks must not silently shorten a malformed key into a valid one.
    !c.is_whitespace()
        && !matches!(
            c,
            '-' | '_'
                | '.'
                | ','
                | ';'
                | ':'
                | '/'
                | '\\'
                | '['
                | ']'
                | '('
                | ')'
                | '{'
                | '}'
                | '–'
                | '—'
        )
}

fn parse(token: &str, explicit: bool) -> Option<String> {
    let mut chars = token.chars();
    let written_note = chars.next()?;
    let note = written_note.to_ascii_uppercase();
    if !('A'..='G').contains(&note) {
        return None;
    }
    let tail = chars.as_str();
    let (accidental, tail) = match tail.chars().next() {
        Some('#' | '♯') => ("#", &tail[tail.chars().next()?.len_utf8()..]),
        Some('b' | '♭') => ("b", &tail[tail.chars().next()?.len_utf8()..]),
        _ => ("", tail),
    };
    // Lowercase natural-note words such as "am" are ordinary title text.
    // A marker/brackets/whole-key filename disambiguates these forms.
    if accidental.is_empty() && !explicit && !written_note.is_ascii_uppercase() {
        return None;
    }
    // An uppercase M is conventionally major; lowercase m is minor.
    let quality = match tail {
        "" if !accidental.is_empty() || explicit => "",
        "M" if explicit || !accidental.is_empty() => "",
        "m" => "m",
        word if word.eq_ignore_ascii_case("min") || word.eq_ignore_ascii_case("minor") => "m",
        word if word.eq_ignore_ascii_case("maj") || word.eq_ignore_ascii_case("major") => "",
        _ => return None,
    };
    Some(format!("{note}{accidental}{quality}"))
}

pub(super) fn from_filename(stem: &str) -> Option<String> {
    let tokens: Vec<(usize, &str)> = stem
        .match_indices(|c: char| !token_char(c))
        .map(|(index, separator)| (index, separator.len()))
        .chain(std::iter::once((stem.len(), 0)))
        .scan(0, |start, (end, separator_len)| {
            let token = (*start, &stem[*start..end]);
            *start = end + separator_len;
            Some(token)
        })
        .filter(|(_, token)| !token.is_empty())
        .collect();
    let mut found = None;
    for (index, &(start, token)) in tokens.iter().enumerate() {
        let marked = index > 0 && tokens[index - 1].1.eq_ignore_ascii_case("key");
        let brackets = matches!(
            (
                stem[..start].chars().next_back(),
                stem[start + token.len()..].chars().next()
            ),
            (Some('['), Some(']')) | (Some('('), Some(')')) | (Some('{'), Some('}'))
        );
        let whole = stem.trim() == token;
        let quality = tokens.get(index + 1).map(|(_, word)| *word).filter(|word| {
            word.eq_ignore_ascii_case("minor") || word.eq_ignore_ascii_case("major")
        });
        let combined;
        let candidate = if let Some(quality) = quality {
            combined = format!("{token}{quality}");
            parse(&combined, true)
        } else {
            parse(token, marked || brackets || whole)
        };
        if marked && candidate.is_none() {
            return None;
        }
        if let Some(key) = candidate {
            if found.as_ref().is_some_and(|previous| previous != &key) {
                return None;
            }
            found = Some(key);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_key_hints_preserve_complete_accidentals_and_qualities() {
        for (input, expected) in [
            ("A#", "A#"),
            ("C#m", "C#m"),
            ("Bb", "Bb"),
            ("Db", "Db"),
            ("Artist - Song A# 124", "A#"),
            ("mix_C#m_128", "C#m"),
            ("Track Bb 110", "Bb"),
            ("Track Dbm", "Dbm"),
            ("Track C♯minor", "C#m"),
            ("Track E♭min", "Ebm"),
            ("Track_F#major", "F#"),
            ("Track_GbM", "Gb"),
            ("Track [C]", "C"),
            ("Track (Dm)", "Dm"),
            ("Track key: a", "A"),
            ("Track C minor", "Cm"),
            ("Track c major", "C"),
            ("  g  ", "G"),
            ("Track C# C♯", "C#"),
        ] {
            assert_eq!(from_filename(input).as_deref(), Some(expected), "{input}");
        }
    }

    #[test]
    fn incidental_and_malformed_filename_tokens_remain_unknown() {
        for input in [
            "",
            "A beautiful day 128",
            "B side 2024",
            "C 120 take 3",
            "ambient bass",
            "I am here 125",
            "I AM HERE 125",
            "abc csharp cMinority 120",
            "99 120 200",
            "Track Hm",
            "Track A##",
            "Track C#x",
            "Track Bbb",
            "Track Db7",
            "Track C#major7",
            "Track C♯♯",
            "Track F𝄪",
            "Track C#mystery",
            "Track A# Bb",
            "Track key: unknown Am",
            "Track key C##",
            "Track A♮",
            "Track Cmajority",
            "Track key F𝄪",
            "Track C#♮",
            "Track [C)",
            "Track {C]",
            "Track key C\u{301}",
            "Track key🦄F",
            "Track C#😀",
        ] {
            assert_eq!(from_filename(input), None, "{input}");
        }
        // Existing BPM hints retain their independent parsing behavior.
        assert_eq!(
            super::super::parse_tags("Artist - C#m - 128"),
            (128.0, "C#m".into())
        );
        assert_eq!(
            super::super::parse_tags("A beautiful day 128"),
            (128.0, "—".into())
        );
    }
}
