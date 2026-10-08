use super::*;

struct Tokens<'a> {
    input: &'a str,
}
impl<'a> Tokens<'a> {
    fn whitespace(&mut self) {
        self.input = self.input.trim_start_matches([' ', '\t', '\r', '\n']);
    }
    fn symbol(&mut self, expected: &str) -> Result<(), String> {
        self.whitespace();
        self.input = self
            .input
            .strip_prefix(expected)
            .ok_or_else(|| format!("Invalid Pack manifest: expected {expected}"))?;
        Ok(())
    }
    fn word(&mut self) -> Result<&'a str, String> {
        self.whitespace();
        let count = self
            .input
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
            .count();
        if count == 0 || count > 128 {
            return Err("Invalid Pack manifest identifier".into());
        }
        let word = &self.input[..count];
        self.input = &self.input[count..];
        Ok(word)
    }
    fn string(&mut self) -> Result<String, String> {
        self.symbol("\"")?;
        let mut value = String::new();
        let mut escape = false;
        for (index, c) in self.input.char_indices() {
            if escape {
                value.push(match c {
                    '"' => '"',
                    '\\' => '\\',
                    _ => return Err("Unsupported Pack string escape".into()),
                });
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                self.input = &self.input[index + 1..];
                return Ok(value);
            } else if c.is_control() {
                return Err("Invalid Pack string character".into());
            } else {
                value.push(c);
            }
            if value.len() > 4096 {
                return Err("Pack value exceeds 4096 bytes".into());
            }
        }
        Err("Truncated Pack string".into())
    }
}
/// Parse the observed installed Pack metadata grammar.
/// Takes bounded Ableton#04I text; returns original declared fields without assuming the manifest grants reuse rights or provides a native engine.
pub(super) fn parse(text: &str) -> Result<BTreeMap<String, String>, String> {
    if text.len() > MAX_MANIFEST {
        return Err("Pack manifest exceeds 256 KiB".into());
    }
    let mut tokens = Tokens { input: text };
    tokens.symbol("Ableton#04I")?;
    tokens.symbol("FolderConfigData")?;
    tokens.symbol("{")?;
    let mut fields = BTreeMap::new();
    loop {
        tokens.whitespace();
        if tokens.input.starts_with('}') {
            tokens.symbol("}")?;
            break;
        }
        if fields.len() >= 128 {
            return Err("Pack manifest exceeds 128 fields".into());
        }
        let kind = tokens.word()?;
        let key = tokens.word()?.to_owned();
        tokens.symbol("=")?;
        let value = match kind {
            "String" => tokens.string()?,
            "Int" => {
                tokens.whitespace();
                let end = tokens
                    .input
                    .bytes()
                    .take_while(|b| b.is_ascii_digit() || *b == b'-')
                    .count();
                if end == 0 || end > 20 {
                    return Err("Invalid Pack integer".into());
                }
                let number = &tokens.input[..end];
                number.parse::<i64>().map_err(|_| "Invalid Pack integer")?;
                tokens.input = &tokens.input[end..];
                number.into()
            }
            _ => return Err("Unsupported Pack field type".into()),
        };
        tokens.symbol(";")?;
        if fields.insert(key, value).is_some() {
            return Err("Pack manifest repeats a field".into());
        }
    }
    tokens.whitespace();
    if !tokens.input.is_empty() {
        return Err("Pack manifest has trailing data".into());
    }
    for required in ["PackUniqueID", "PackDisplayName", "PackVendor"] {
        if fields.get(required).is_none_or(|s| s.is_empty()) {
            return Err(format!("Installed Pack has no {required}"));
        }
    }
    for name in ["PackMajorVersion", "PackMinorVersion", "PackRevision"] {
        if fields.get(name).is_none_or(|s| s.parse::<u32>().is_err()) {
            return Err(format!(
                "Installed Pack needs a declared nonnegative {name}"
            ));
        }
    }
    Ok(fields)
}
