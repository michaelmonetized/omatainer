use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::collections::BTreeMap;

const MAX_NODES: usize = 250_000;
const MAX_TEXT: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct Element {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub children: Vec<Element>,
}
impl Element {
    /// Read an optional attribute. Takes its exact XML name; returns an empty string when absent.
    pub fn attr(&self, key: &str) -> &str {
        self.attributes.get(key).map(String::as_str).unwrap_or("")
    }
    /// Read one structural child. Takes its exact XML name; refuses missing or repeated containers.
    pub fn one(&self, key: &str) -> Result<&Element, String> {
        let mut children = self.children.iter().filter(|child| child.name == key);
        let first = children
            .next()
            .ok_or_else(|| format!("{} has no {key}", self.name))?;
        if children.next().is_some() {
            return Err(format!("{} repeats {key}", self.name));
        }
        Ok(first)
    }
    /// Iterate children in source order. Takes their exact XML name; returns matching elements without allocation.
    pub fn named<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |child| child.name == key)
    }
}

struct Budget {
    nodes: usize,
    text: usize,
}
impl Budget {
    fn element(
        &mut self,
        start: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
    ) -> Result<Element, String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err("XML exceeds 250000 elements".into());
        }
        let name = std::str::from_utf8(start.name().as_ref())
            .map_err(|_| "XML names must be UTF-8")?
            .to_owned();
        if name.len() > 128 || name.contains(':') {
            return Err("Unsupported oversized or namespaced XML element".into());
        }
        let mut attributes = BTreeMap::new();
        for attribute in start.attributes() {
            let attribute = attribute.map_err(|e| e.to_string())?;
            let key = std::str::from_utf8(attribute.key.as_ref())
                .map_err(|_| "XML attributes must be UTF-8")?
                .to_owned();
            let value = attribute
                .decode_and_unescape_value(reader.decoder())
                .map_err(|e| e.to_string())?
                .into_owned();
            self.text = self
                .text
                .checked_add(key.len() + value.len())
                .ok_or("XML text budget exceeded")?;
            if attributes.len() >= 64
                || key.len() > 128
                || value.len() > 4096
                || self.text > MAX_TEXT
                || value
                    .chars()
                    .any(|c| matches!(c as u32, 0..=8|11..=12|14..=31|0xfffe|0xffff))
            {
                return Err(
                    "XML exceeds its attribute/text budget or contains invalid characters".into(),
                );
            }
            if attributes.insert(key, value).is_some() {
                return Err("Duplicate XML attribute".into());
            }
        }
        Ok(Element {
            name,
            attributes,
            children: Vec::new(),
        })
    }
}

/// Parse attribute-based interchange XML without external resources.
/// Takes UTF-8 bytes and a cancellation predicate; returns a bounded ordered tree or rejects DTDs, text, truncation and invalid nesting.
pub(crate) fn parse(bytes: &[u8], active: &impl Fn() -> bool) -> Result<Element, String> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("XML exceeds 32 MiB".into());
    }
    let source = std::str::from_utf8(bytes)
        .map_err(|_| "Interchange XML requires UTF-8")?
        .trim_start_matches('\u{feff}');
    let mut reader = Reader::from_str(source);
    let mut budget = Budget { nodes: 0, text: 0 };
    let mut stack: Vec<Element> = Vec::new();
    let mut root = None;
    let mut declared = false;
    let mut events = 0;
    loop {
        if !active() {
            return Err("XML import cancelled".into());
        }
        events += 1;
        if events > 750_000 {
            return Err("XML exceeds 750000 parser events".into());
        }
        let element = match reader
            .read_event()
            .map_err(|e| format!("Invalid XML: {e}"))?
        {
            Event::Decl(declaration) if !declared && root.is_none() && stack.is_empty() => {
                declared = true;
                if declaration.version().map_err(|e| e.to_string())?.as_ref() != b"1.0"
                    || declaration
                        .encoding()
                        .transpose()
                        .map_err(|e| e.to_string())?
                        .is_some_and(|e| !e.eq_ignore_ascii_case(b"UTF-8"))
                {
                    return Err("Interchange XML requires XML 1.0 and UTF-8".into());
                }
                continue;
            }
            Event::Start(start) => {
                if stack.len() >= 64 || root.is_some() {
                    return Err("XML exceeds 64 levels or has multiple roots".into());
                }
                stack.push(budget.element(&start, &reader)?);
                continue;
            }
            Event::Empty(start) => {
                if root.is_some() {
                    return Err("XML has multiple roots".into());
                }
                budget.element(&start, &reader)?
            }
            Event::End(end) => {
                let element = stack.pop().ok_or("Unexpected XML closing element")?;
                if end.name().as_ref() != element.name.as_bytes() {
                    return Err("Mismatched XML closing element".into());
                }
                element
            }
            Event::Comment(comment) => {
                budget.text += comment.len();
                if budget.text > MAX_TEXT {
                    return Err("XML exceeds its text budget".into());
                }
                continue;
            }
            Event::Text(text) if text.decode().map_err(|e| e.to_string())?.trim().is_empty() => {
                continue
            }
            Event::Eof => {
                if !stack.is_empty() {
                    return Err("Truncated XML".into());
                }
                return root.ok_or("XML has no root element".into());
            }
            _ => return Err(
                "Interchange XML forbids DTDs, entities, processing instructions and text content"
                    .into(),
            ),
        };
        if let Some(parent) = stack.last_mut() {
            parent.children.push(element);
        } else if root.replace(element).is_some() {
            return Err("XML has multiple roots".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_or_incomplete_xml_is_refused_and_unicode_entities_decode() {
        let tree = parse(
            b"<ROOT Name=\"Bj\xc3\xb6rk &amp; \xe6\x9d\xb1\xe4\xba\xac\"><X/></ROOT>",
            &|| true,
        )
        .unwrap();
        assert_eq!(tree.attr("Name"), "Björk & 東京");
        for invalid in [
            "<X>",
            "<X/><Y/>",
            "<X><Y></X></Y>",
            "<!DOCTYPE X SYSTEM 'file:///etc/passwd'><X/>",
            "<X Name='&unknown;'/>",
            "<X Name='one' Name='two'/>",
            "<X>text</X>",
            "<?xml version='1.0' encoding='UTF-16'?><X/>",
        ] {
            assert!(parse(invalid.as_bytes(), &|| true).is_err(), "{invalid}");
        }
        assert!(parse(
            format!("{}{}", "<X>".repeat(65), "</X>".repeat(65)).as_bytes(),
            &|| true
        )
        .is_err());
        assert!(parse(b"<X/>", &|| false).is_err());
    }
}
