use quick_xml::{XmlVersion, events::Event, name::ResolveResult, reader::NsReader};
use std::{cmp::Ordering, collections::HashSet};

const SPARKLE: &[u8] = b"http://www.andymatuschak.org/xml-namespaces/sparkle";
#[derive(Clone, Debug, Default)]
pub struct Enclosure {
    pub url: String,
    pub length: Option<u64>,
    pub signature: String,
    pub dsa_signature: String,
    pub arguments: String,
    pub os: String,
    pub version: String,
    pub short_version: String,
}
#[derive(Clone, Debug, Default)]
pub struct Release {
    pub version: String,
    pub short_version: String,
    pub title: String,
    pub description: String,
    pub notes_url: String,
    pub web_url: String,
    pub min_os: String,
    pub critical: bool,
    pub enclosure: Option<Enclosure>,
}
impl Release {
    pub fn display_version(&self) -> &str {
        if self.short_version.is_empty() {
            &self.version
        } else {
            &self.short_version
        }
    }
}
fn namespace(result: ResolveResult<'_>) -> crate::Result<Vec<u8>> {
    match result {
        ResolveResult::Unbound => Ok(Vec::new()),
        ResolveResult::Bound(v) => Ok(v.as_ref().as_bytes().to_vec()),
        ResolveResult::Unknown(_) => Err("Undeclared XML namespace".into()),
    }
}

pub fn parse(xml: &[u8], architecture: &str) -> crate::Result<Vec<Release>> {
    if xml.len() > 4 * 1024 * 1024 {
        return Err("Appcast exceeds 4 MiB".into());
    }
    let mut reader = NsReader::from_reader(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack: Vec<(Vec<u8>, String, String)> = Vec::new();
    let mut releases = Vec::new();
    let mut item: Option<Release> = None;
    let mut item_depth = 0;
    let mut enclosures = Vec::new();
    let mut roots = 0;
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("Invalid appcast XML: {e}"))?
        {
            Event::Start(e) => {
                let (ns, name) = reader.resolver().resolve_element(e.name());
                let ns = namespace(ns)?;
                let name = name.as_ref().to_owned();
                if stack.len() >= 64 {
                    return Err("XML nesting exceeds 64 levels".into());
                }
                if stack.is_empty() {
                    roots += 1;
                    if roots != 1 || name != "rss" {
                        return Err("Expected one RSS document".into());
                    }
                }
                let mut attributes = Vec::new();
                let mut seen = HashSet::new();
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| format!("Invalid XML attribute: {e}"))?;
                    if attr.key.as_ref() == "xmlns" || attr.key.as_ref().starts_with("xmlns:") {
                        continue;
                    }
                    let (a_ns, local) = reader.resolver().resolve_attribute(attr.key);
                    let a_ns = namespace(a_ns)?;
                    let key = local.as_ref().to_owned();
                    if !seen.insert((a_ns.clone(), key.clone())) {
                        return Err("Duplicate expanded XML attribute".into());
                    }
                    let value = attr
                        .normalized_value(XmlVersion::Implicit1_0)
                        .map_err(|e| e.to_string())?
                        .into_owned();
                    attributes.push((a_ns, key, value));
                }
                if ns.is_empty() && name == "item" && stack.last().is_some_and(|p| p.1 == "channel")
                {
                    if item.is_some() {
                        return Err("Nested RSS item".into());
                    }
                    item = Some(Release::default());
                    item_depth = stack.len();
                    enclosures.clear();
                } else if let Some(release) = item.as_mut()
                    && stack.len() == item_depth + 1
                {
                    if ns.is_empty() && name == "enclosure" {
                        let mut enclosure = Enclosure::default();
                        for (ns, key, value) in attributes {
                            match (ns.as_slice(), key.as_str()) {
                                (b"", "url") => enclosure.url = value,
                                (b"", "length") => {
                                    if !value.is_empty() {
                                        enclosure.length = Some(
                                            value
                                                .parse()
                                                .map_err(|_| "Invalid enclosure length")?,
                                        );
                                    }
                                }
                                (SPARKLE, "version") => enclosure.version = value,
                                (SPARKLE, "shortVersionString") => enclosure.short_version = value,
                                (SPARKLE, "edSignature") => enclosure.signature = value,
                                (SPARKLE, "dsaSignature") => enclosure.dsa_signature = value,
                                (SPARKLE, "installerArguments") => enclosure.arguments = value,
                                (SPARKLE, "os") => enclosure.os = value,
                                _ => {}
                            }
                        }
                        enclosures.push(enclosure);
                    } else if ns == SPARKLE && name == "criticalUpdate" {
                        release.critical = true;
                    }
                }
                stack.push((ns, name, String::new()));
            }
            Event::Text(text) => {
                let text = text.xml_content(XmlVersion::Implicit1_0);
                if let Some(node) = stack.last_mut() {
                    node.2.push_str(&text);
                } else if !text.trim().is_empty() {
                    return Err("Text outside XML root".into());
                }
            }
            Event::CData(text) => {
                if let Some(node) = stack.last_mut() {
                    node.2.push_str(&text);
                }
            }
            Event::End(_) => {
                let Some((ns, name, text)) = stack.pop() else {
                    return Err("Unexpected closing XML tag".into());
                };
                if let Some(release) = item.as_mut() {
                    if stack.len() == item_depth + 1 {
                        let text = text.trim().to_owned();
                        match (ns.as_slice(), name.as_str()) {
                            (b"", "title") => release.title = text,
                            (b"", "description") => release.description = readable_notes(&text),
                            (b"", "link") => release.web_url = text,
                            (SPARKLE, "version") => release.version = text,
                            (SPARKLE, "shortVersionString") => release.short_version = text,
                            (SPARKLE, "releaseNotesLink") => release.notes_url = text,
                            (SPARKLE, "minimumSystemVersion") => release.min_os = text,
                            _ => {}
                        }
                    } else if ns.is_empty() && name == "item" && stack.len() == item_depth {
                        let mut release = item.take().unwrap();
                        let had_enclosures = !enclosures.is_empty();
                        let arch_marker = format!("windows-{architecture}");
                        enclosures.retain(|e| {
                            e.os.is_empty() || e.os == "windows" || e.os == arch_marker
                        });
                        enclosures.sort_by_key(|e| {
                            if e.os == arch_marker {
                                0
                            } else if e.os == "windows" {
                                1
                            } else {
                                2
                            }
                        });
                        release.enclosure = enclosures.first().cloned();
                        if let Some(e) = release.enclosure.as_ref() {
                            if release.version.is_empty() {
                                release.version.clone_from(&e.version);
                            }
                            if release.short_version.is_empty() {
                                release.short_version.clone_from(&e.short_version);
                            }
                            if !http_url(&e.url) {
                                return Err("Invalid enclosure URL".into());
                            }
                        }
                        if !release.notes_url.is_empty() && !http_url(&release.notes_url) {
                            return Err("Invalid release notes URL".into());
                        }
                        if !release.web_url.is_empty() && !http_url(&release.web_url) {
                            return Err("Invalid download page URL".into());
                        }
                        if !release.version.is_empty()
                            && (!had_enclosures || release.enclosure.is_some())
                            && (release.enclosure.is_some() || !release.web_url.is_empty())
                        {
                            releases.push(release);
                        }
                    }
                }
                if item.is_some()
                    && stack.len() > item_depth + 1
                    && let Some(parent) = stack.last_mut()
                {
                    parent.2.push_str(&text);
                }
            }
            Event::DocType(_) => return Err("DTD and external entities are not supported".into()),
            Event::GeneralRef(reference) => {
                let encoded = format!("&{};", &*reference);
                let resolved = quick_xml::escape::unescape(&encoded).map_err(|e| e.to_string())?;
                if let Some(node) = stack.last_mut() {
                    node.2.push_str(&resolved);
                } else {
                    return Err("Entity outside XML root".into());
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if roots != 1 || !stack.is_empty() {
        return Err("Incomplete appcast XML".into());
    }
    releases.sort_by(|a, b| compare_versions(&b.version, &a.version));
    Ok(releases)
}
pub fn http_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && !url.contains(['\0', '\r', '\n'])
        && url
            .split_once("://")
            .is_some_and(|(_, s)| !s.is_empty() && !s.starts_with('/'))
}
pub fn readable_notes(html: &str) -> String {
    let mut result = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    let mut hidden = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            tag.clear();
        } else if c == '>' && in_tag {
            in_tag = false;
            let t = tag.trim().to_ascii_lowercase();
            if t.starts_with("script") || t.starts_with("style") {
                hidden = true;
            }
            if t.starts_with("/script") || t.starts_with("/style") {
                hidden = false;
            }
            if ["p", "/p", "br", "br/", "/li", "/h1", "/h2", "/h3"].contains(&t.as_str()) {
                result.push('\n');
            }
            if t == "li" {
                result.push_str("• ");
            }
        } else if in_tag {
            tag.push(c);
        } else if !hidden {
            result.push(c);
        }
    }
    let result = result.replace("&nbsp;", " ");
    let result =
        quick_xml::escape::unescape(&result).unwrap_or(std::borrow::Cow::Borrowed(&result));
    result
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn kind(b: u8) -> u8 {
    if b == b'.' {
        0
    } else if b.is_ascii_digit() {
        1
    } else {
        2
    }
}
fn parts(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut starts = vec![0];
    for i in 1..bytes.len() {
        if kind(bytes[i]) != kind(bytes[i - 1]) || kind(bytes[i]) == 0 {
            starts.push(i);
        }
    }
    starts.push(bytes.len());
    starts
        .windows(2)
        .filter(|p| p[0] != p[1])
        .map(|p| &s[p[0]..p[1]])
        .collect()
}
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let a = parts(a);
    let b = parts(b);
    for (a, b) in a.iter().zip(&b) {
        let (ka, kb) = (kind(a.as_bytes()[0]), kind(b.as_bytes()[0]));
        let result = if ka == kb {
            match ka {
                1 => {
                    let a = a.trim_start_matches('0');
                    let b = b.trim_start_matches('0');
                    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
                }
                2 => a.cmp(b),
                _ => Ordering::Equal,
            }
        } else if ka == 2 {
            Ordering::Less
        } else if kb == 2 {
            Ordering::Greater
        } else {
            ka.cmp(&kb)
        };
        if result != Ordering::Equal {
            return result;
        }
    }
    let n = a.len().min(b.len());
    if a.len() == b.len() {
        Ordering::Equal
    } else if a.len() > b.len() {
        if kind(a[n].as_bytes()[0]) == 2 {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    } else if kind(b[n].as_bytes()[0]) == 2 {
        Ordering::Greater
    } else {
        Ordering::Less
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparkle_versions() {
        for (a, b) in [
            ("1.2rc1", "1.2"),
            ("1.2rc2", "1.2rc10"),
            ("1.5", "1.5.1"),
            ("1.2b3", "1.2rc1"),
            ("99", "100"),
            ("999999999999999999999", "1000000000000000000000"),
        ] {
            assert_eq!(compare_versions(a, b), Ordering::Less, "{a} < {b}");
            assert_eq!(compare_versions(b, a), Ordering::Greater);
        }
        assert_eq!(compare_versions("1.02", "1.2"), Ordering::Equal);
    }
    #[test]
    fn namespaced_cdata_architecture() {
        let xml = br#"<rss xmlns:s="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item><description><![CDATA[<p>New &amp; better</p><script>hidden</script>]]></description><s:criticalUpdate/><enclosure url="https://example.org/a.exe" s:version="20" s:os="windows-x64" s:shortVersionString="2.0"/><enclosure url="https://example.org/b.exe" s:version="21" s:os="macos"/></item></channel></rss>"#;
        let releases = parse(xml, "x64").unwrap();
        assert_eq!(releases[0].version, "20");
        assert!(releases[0].critical);
        assert_eq!(releases[0].description, "New & better");
        assert!(parse(xml, "arm64").unwrap().is_empty());
    }
    #[test]
    fn rejects_bad_xml() {
        for xml in [
            "<rss>",
            "<rss/><rss/>",
            "<!DOCTYPE rss><rss/>",
            "<rss><channel><item><enclosure url='a' url='b'/></item></channel></rss>",
            "<rss><channel></rss>",
        ] {
            assert!(parse(xml.as_bytes(), "x64").is_err(), "{xml}");
        }
    }
    #[test]
    fn namespace_spoof_does_not_supply_version() {
        assert!(parse(br#"<rss xmlns:s="evil"><channel><item><s:version>9</s:version><link>https://example.org</link></item></channel></rss>"#,"x64").unwrap().is_empty());
    }
}
