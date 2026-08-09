use quick_xml::{events::Event, Reader};
use std::collections::HashMap;
#[derive(Debug, Clone)]
pub struct XmlElement {
    pub tag: String,
    pub attributes: HashMap<String, String>,
    pub text: String,
    pub children: Vec<XmlElement>,
}
impl XmlElement {
    pub fn get_val(&self, key: &str) -> Option<String> {
        if let Some((_, v)) = self
            .attributes
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
        {
            return Some(v.clone());
        }
        for child in &self.children {
            if child.tag.eq_ignore_ascii_case(key) {
                if !child.text.is_empty() {
                    return Some(child.text.clone());
                }
            }
        }
        None
    }
    pub fn get_list(&self, key: &str) -> Vec<XmlElement> {
        let mut res = Vec::new();
        for child in &self.children {
            if child.tag.eq_ignore_ascii_case(key) {
                let mut has_li = false;
                for c in &child.children {
                    if c.tag.eq_ignore_ascii_case("li")
                        || c.tag.eq_ignore_ascii_case("Item")
                        || c.tag.eq_ignore_ascii_case("Stuff")
                    {
                        res.push(c.clone());
                        has_li = true;
                    }
                }
                if !has_li {
                    res.push(child.clone());
                }
            }
        }
        res
    }
}
pub fn parse_xml(xml: &str) -> Vec<XmlElement> {
    let safe_xml = xml.trim_start_matches(|c: char| c == '\u{FEFF}' || c.is_whitespace());
    let mut clean_xml = String::with_capacity(safe_xml.len());
    let mut chars = safe_xml.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let (mut tag_preview, mut clone_chars) = (String::new(), chars.clone());
            for _ in 0..15 {
                if let Some(nc) = clone_chars.next() {
                    tag_preview.push(nc);
                    if nc == '>' {
                        break;
                    }
                }
            }
            let tag_lower = tag_preview.to_lowercase();
            if tag_lower.starts_with("color=")
                || tag_lower.starts_with("size=")
                || tag_lower.starts_with("align=")
                || tag_lower.starts_with("/color>")
                || tag_lower.starts_with("/size>")
                || tag_lower.starts_with("/align>")
            {
                while let Some(skip_c) = chars.next() {
                    if skip_c == '>' {
                        break;
                    }
                }
                continue;
            }
        }
        if c == '&' {
            let (mut clone_chars, mut is_entity) = (chars.clone(), false);
            for _ in 0..10 {
                if let Some(nc) = clone_chars.next() {
                    if nc == ';' {
                        is_entity = true;
                        break;
                    }
                    if nc == ' ' || nc == '<' {
                        break;
                    }
                }
            }
            if !is_entity {
                clean_xml.push_str("&amp;");
                continue;
            }
        }
        clean_xml.push(c);
    }
    let mut reader = Reader::from_str(&clean_xml);
    reader.check_end_names(false);
    reader.trim_text(true);
    let (mut buf, mut stack, mut roots) = (
        Vec::new(),
        Vec::<XmlElement>::new(),
        Vec::<XmlElement>::new(),
    );
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag_raw = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let tag = tag_raw.split(':').last().unwrap_or(&tag_raw).to_string();
                let mut attributes = HashMap::new();
                for attr_res in e.attributes() {
                    if let Ok(attr) = attr_res {
                        let key_raw = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                        let key = key_raw.split(':').last().unwrap_or(&key_raw).to_string();
                        let val = attr
                            .decode_and_unescape_value(&reader)
                            .map(|c| c.into_owned())
                            .unwrap_or_else(|_| {
                                String::from_utf8_lossy(attr.value.as_ref()).into_owned()
                            });
                        attributes.insert(key, val);
                    }
                }
                let node = XmlElement {
                    tag,
                    attributes,
                    text: String::new(),
                    children: Vec::new(),
                };
                stack.push(node);
            }
            Ok(Event::Empty(ref e)) => {
                let tag_raw = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let tag = tag_raw.split(':').last().unwrap_or(&tag_raw).to_string();
                let mut attributes = HashMap::new();
                for attr_res in e.attributes() {
                    if let Ok(attr) = attr_res {
                        let key_raw = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                        let key = key_raw.split(':').last().unwrap_or(&key_raw).to_string();
                        let val = attr
                            .decode_and_unescape_value(&reader)
                            .map(|c| c.into_owned())
                            .unwrap_or_else(|_| {
                                String::from_utf8_lossy(attr.value.as_ref()).into_owned()
                            });
                        attributes.insert(key, val);
                    }
                }
                let node = XmlElement {
                    tag,
                    attributes,
                    text: String::new(),
                    children: Vec::new(),
                };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    roots.push(node);
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e
                    .unescape()
                    .map(|c| c.into_owned())
                    .unwrap_or_else(|_| String::from_utf8_lossy(e.as_ref()).into_owned());
                let text_trimmed = text.trim();
                if !text_trimmed.is_empty() {
                    if let Some(last) = stack.last_mut() {
                        if !last.text.is_empty() {
                            last.text.push(' ');
                        }
                        last.text.push_str(text_trimmed);
                    }
                }
            }
            Ok(Event::CData(ref e)) => {
                let text = String::from_utf8_lossy(e.as_ref()).into_owned();
                let text_trimmed = text.trim();
                if !text_trimmed.is_empty() {
                    if let Some(last) = stack.last_mut() {
                        if !last.text.is_empty() {
                            last.text.push(' ');
                        }
                        last.text.push_str(text_trimmed);
                    }
                }
            }
            Ok(Event::End(_)) => {
                if let Some(mut node) = stack.pop() {
                    node.text = node.text.trim().to_string();
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    } else {
                        roots.push(node);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => (),
        }
        buf.clear();
    }
    while let Some(mut node) = stack.pop() {
        node.text = node.text.trim().to_string();
        if let Some(parent) = stack.last_mut() {
            parent.children.push(node);
        } else {
            roots.push(node);
        }
    }
    roots
}
