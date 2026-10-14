use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlElement {
    pub tag: String,
    pub attributes: HashMap<String, String>,
    pub text: String,
    pub children: Vec<XmlElement>,
}

impl XmlElement {
    pub fn child(&self, key: &str) -> Option<&Self> {
        self.children
            .iter()
            .find(|child| child.tag.eq_ignore_ascii_case(key))
    }

    /// Presence matters: an explicit empty value must override an inherited one.
    pub fn get_val(&self, key: &str) -> Option<String> {
        self.attributes
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.clone())
            .or_else(|| self.child(key).map(|child| child.text.clone()))
    }

    pub fn get_list(&self, key: &str) -> Vec<Self> {
        let mut result = Vec::new();
        for child in self
            .children
            .iter()
            .filter(|child| child.tag.eq_ignore_ascii_case(key))
        {
            // A list entry may itself contain a scalar <Stuff>; do not flatten it.
            if key.eq_ignore_ascii_case("li") {
                result.push(child.clone());
                continue;
            }
            let entries: Vec<_> = child
                .children
                .iter()
                .filter(|entry| {
                    ["li", "Item", "Stuff"]
                        .iter()
                        .any(|tag| entry.tag.eq_ignore_ascii_case(tag))
                })
                .cloned()
                .collect();
            if entries.is_empty() {
                result.push(child.clone());
            } else {
                result.extend(entries);
            }
        }
        result
    }

    /// Document order is also the order of the game's random-selection pools.
    pub fn descendants(&self) -> impl Iterator<Item = &Self> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            stack.extend(node.children.iter().rev());
            Some(node)
        })
    }
}

fn local_name(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn node_from_start(event: &BytesStart<'_>, reader: &Reader<&[u8]>) -> Result<XmlElement, String> {
    let mut attributes = HashMap::new();
    for attribute in event.attributes() {
        let attribute = attribute.map_err(|error| format!("invalid XML attribute: {error}"))?;
        let value = attribute
            .decode_and_unescape_value(reader)
            .map_err(|error| format!("invalid XML attribute value: {error}"))?
            .into_owned();
        attributes.insert(local_name(attribute.key.as_ref()), value);
    }
    Ok(XmlElement {
        tag: local_name(event.name().as_ref()),
        attributes,
        text: String::new(),
        children: Vec::new(),
    })
}

// Some official descriptions contain Unity rich-text tags and bare ampersands.
// Normalize only these known extensions, then parse the structural XML strictly.
fn normalize_xml(xml: &str) -> String {
    let source = xml.trim_start_matches(|c: char| c == '\u{feff}' || c.is_whitespace());
    let mut result = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut rich_tags = HashMap::new();
    while let Some(character) = chars.next() {
        if character == '<'
            && rich_text_tag(&chars.clone().take(32).collect::<String>(), &mut rich_tags)
        {
            skip_tag(&mut chars);
            continue;
        }
        if character == '&' && !recognized_entity(chars.clone()) {
            result.push_str("&amp;");
        } else {
            result.push(character);
        }
    }
    result
}

fn rich_text_tag(preview: &str, rich_tags: &mut HashMap<&'static str, usize>) -> bool {
    let lower = preview.to_ascii_lowercase();
    let tags = ["color", "size", "align"];
    let opening = tags
        .into_iter()
        .find(|tag| lower.starts_with(&format!("{tag}=")));
    let closing = tags.into_iter().find(|tag| {
        lower.starts_with(&format!("/{tag}>")) && rich_tags.get(tag).copied().unwrap_or(0) > 0
    });
    if let Some(tag) = opening {
        *rich_tags.entry(tag).or_default() += 1;
    }
    if let Some(count) = closing.and_then(|tag| rich_tags.get_mut(tag)) {
        *count -= 1;
    }
    opening.is_some() || closing.is_some()
}

fn skip_tag(chars: &mut impl Iterator<Item = char>) {
    for next in chars {
        if next == '>' {
            break;
        }
    }
}

fn recognized_entity(chars: impl Iterator<Item = char>) -> bool {
    let tail: String = chars
        .take(16)
        .take_while(|c| *c != ';' && !c.is_whitespace() && *c != '<')
        .collect();
    ["amp", "lt", "gt", "quot", "apos"].contains(&tail.as_str())
        || tail.strip_prefix("#x").is_some_and(|number| {
            !number.is_empty() && number.chars().all(|c| c.is_ascii_hexdigit())
        })
        || tail
            .strip_prefix('#')
            .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

#[derive(Default)]
struct XmlDocument {
    stack: Vec<XmlElement>,
    roots: Vec<XmlElement>,
}

impl XmlDocument {
    fn append_node(&mut self, node: XmlElement) {
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(node);
        } else {
            self.roots.push(node);
        }
    }

    fn append_text(&mut self, value: &str) -> Result<(), String> {
        if let Some(node) = self.stack.last_mut() {
            node.text.push_str(value);
        } else if !value.trim().is_empty() {
            return Err("text outside XML root".into());
        }
        Ok(())
    }

    fn close_node(&mut self) -> Result<(), String> {
        let mut node = self.stack.pop().ok_or("unexpected XML closing tag")?;
        let leading = node.text.len() - node.text.trim_start().len();
        node.text.drain(..leading);
        node.text.truncate(node.text.trim_end().len());
        self.append_node(node);
        Ok(())
    }

    fn read_event(&mut self, event: Event<'_>, reader: &Reader<&[u8]>) -> Result<(), String> {
        match event {
            Event::Start(start) => self.stack.push(node_from_start(&start, reader)?),
            Event::Empty(start) => self.append_node(node_from_start(&start, reader)?),
            Event::End(_) => self.close_node()?,
            event => self.read_content(event)?,
        }
        Ok(())
    }

    fn read_content(&mut self, event: Event<'_>) -> Result<(), String> {
        match event {
            Event::Text(text) => {
                let value = text
                    .unescape()
                    .map_err(|error| format!("invalid XML text: {error}"))?;
                self.append_text(&value)?;
            }
            Event::CData(text) => {
                let value = std::str::from_utf8(text.as_ref())
                    .map_err(|error| format!("invalid UTF-8 CDATA: {error}"))?;
                if let Some(node) = self.stack.last_mut() {
                    node.text.push_str(value);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<Vec<XmlElement>, String> {
        if !self.stack.is_empty() {
            return Err("unclosed XML element".into());
        }
        if self.roots.is_empty() {
            return Err("XML contains no root element".into());
        }
        Ok(self.roots)
    }
}

pub fn parse_xml_checked(xml: &str) -> Result<Vec<XmlElement>, String> {
    let normalized = normalize_xml(xml);
    let mut reader = Reader::from_str(&normalized);
    reader.trim_text(false);
    let mut document = XmlDocument::default();
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("XML at byte {}: {error}", reader.buffer_position()))?;
        if matches!(event, Event::Eof) {
            break;
        }
        document.read_event(event, &reader)?;
    }
    document.finish()
}
