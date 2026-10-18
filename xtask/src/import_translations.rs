use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    path::Path,
};

pub fn parse_mo(bytes: &[u8]) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    if bytes.len() < 28 {
        return Err("Truncated MO header".into());
    }
    let magic = u32::from_le_bytes(bytes[..4].try_into()?);
    let little = match magic {
        0x950412de => true,
        0xde120495 => false,
        _ => return Err("Invalid MO magic".into()),
    };
    let read = |offset: usize| -> Result<usize, Box<dyn Error>> {
        let b: [u8; 4] = bytes
            .get(offset..offset.checked_add(4).ok_or("MO offset overflow")?)
            .ok_or("MO offset out of bounds")?
            .try_into()?;
        Ok(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        } as usize)
    };
    if read(4)? >> 16 != 0 {
        return Err("Unsupported MO major revision".into());
    }
    let count = read(8)?;
    let original = read(12)?;
    let translated = read(16)?;
    let table_size = count.checked_mul(8).ok_or("MO table overflow")?;
    for start in [original, translated] {
        bytes
            .get(start..start.checked_add(table_size).ok_or("MO table overflow")?)
            .ok_or("MO table out of bounds")?;
    }
    let string = |table: usize, index: usize| -> Result<String, Box<dyn Error>> {
        let size = read(table + index * 8)?;
        let offset = read(table + index * 8 + 4)?;
        let end = offset.checked_add(size).ok_or("MO string overflow")?;
        if bytes.get(end) != Some(&0) {
            return Err("MO string has no NUL terminator".into());
        }
        Ok(
            std::str::from_utf8(bytes.get(offset..end).ok_or("MO string out of bounds")?)?
                .to_owned(),
        )
    };
    let mut result = BTreeMap::new();
    for i in 0..count {
        let key = string(original, i)?;
        let value = string(translated, i)?;
        if result.insert(key, value).is_some() {
            return Err("Duplicate MO message".into());
        }
    }
    if let Some(header) = result.get("")
        && let Some(charset) = header.to_ascii_lowercase().split("charset=").nth(1)
        && !charset.starts_with("utf-8")
    {
        return Err("MO catalogue is not UTF-8; refusing lossy conversion".into());
    }
    Ok(result)
}
fn locale_id(locale: &str) -> Option<u16> {
    let normalized = locale.replace('_', "-");
    let base = normalized.split('-').next().unwrap_or("");
    Some(match normalized.as_str() {
        "pt-BR" => 0x0416,
        "zh-TW" => 0x0404,
        "zh-CN" => 0x0804,
        "sr@latin" => 0x081a,
        _ => match base {
            "af" => 0x0436,
            "ar" => 0x0401,
            "be" => 0x0423,
            "bg" => 0x0402,
            "bs" => 0x141a,
            "ca" => 0x0403,
            "co" => 0x0483,
            "cs" => 0x0405,
            "da" => 0x0406,
            "de" => 0x0407,
            "el" => 0x0408,
            "en" => 0x0409,
            "es" => 0x040a,
            "et" => 0x0425,
            "eu" => 0x042d,
            "fa" => 0x0429,
            "fi" => 0x040b,
            "fr" => 0x040c,
            "fy" => 0x0462,
            "ga" => 0x083c,
            "gl" => 0x0456,
            "he" => 0x040d,
            "hi" => 0x0439,
            "hr" => 0x041a,
            "hu" => 0x040e,
            "id" => 0x0421,
            "is" => 0x040f,
            "it" => 0x0410,
            "ja" => 0x0411,
            "ka" => 0x0437,
            "kk" => 0x043f,
            "hy" => 0x042b,
            "ko" => 0x0412,
            "lt" => 0x0427,
            "lv" => 0x0426,
            "ms" => 0x043e,
            "nb" | "no" => 0x0414,
            "nl" => 0x0413,
            "nn" => 0x0814,
            "pl" => 0x0415,
            "pt" => 0x0816,
            "ro" => 0x0418,
            "ru" => 0x0419,
            "sk" => 0x041b,
            "sl" => 0x0424,
            "sq" => 0x041c,
            "sr" => 0x0c1a,
            "sv" => 0x041d,
            "ta" => 0x0449,
            "th" => 0x041e,
            "tr" => 0x041f,
            "ug" => 0x0480,
            "uk" => 0x0422,
            "uz" => 0x0443,
            "vi" => 0x042a,
            "zh" => 0x0804,
            _ => return None,
        },
    })
}
fn rc_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\"\"")
        .replace('\r', "")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}
fn block(lang: u16, strings: &BTreeMap<String, String>) -> Result<String, Box<dyn Error>> {
    let mut out = format!(
        "LANGUAGE {}, {}\nSTRINGTABLE\nBEGIN\n",
        lang & 0x3ff,
        lang >> 10
    );
    let mut ordered = Vec::new();
    for (id, text) in strings {
        ordered.push((id.parse::<u16>()?, text));
    }
    ordered.sort_by_key(|(id, _)| *id);
    for (id, text) in ordered {
        if text.encode_utf16().count() > 4096 || text.contains('\0') {
            return Err(format!("String {id} is not representable in a resource").into());
        }
        out.push_str(&format!("    {id}, \"{}\"\n", rc_escape(text)));
    }
    out.push_str("END\n\n");
    Ok(out)
}
pub fn run(source: &Path) -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let english: BTreeMap<String, String> = toml::from_str(&std::fs::read_to_string(
        root.join("resources/strings.toml"),
    )?)?;
    let mut output =
        String::from("// Generated by cargo xtask import-translations. Do not edit.\n");
    let mut report = String::from(
        "# Translation import report\n\nSource: WinSparkle gettext MO catalogues (UTF-8). Context and plural entries are parsed, retained in diagnostics, and never flattened into unrelated UI messages. Missing messages use English.\n\n| Locale | LANGID | Imported | Overrides | Missing | Unmapped |\n|---|---:|---:|---:|---:|---:|\n",
    );
    let mut files: Vec<_> = std::fs::read_dir(source)?.collect::<Result<_, _>>()?;
    files.sort_by_key(|f| f.file_name());
    let mut langs = BTreeSet::new();
    let mut details = String::new();
    for entry in files {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("mo") {
            continue;
        }
        let locale = path
            .file_stem()
            .unwrap()
            .to_str()
            .ok_or("Invalid locale filename")?;
        let lang = locale_id(locale).ok_or_else(|| format!("Unmapped locale: {locale}"))?;
        if !langs.insert(lang) {
            return Err(format!("Duplicate LANGID for {locale}").into());
        }
        let messages =
            parse_mo(&std::fs::read(&path)?).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut strings = english.clone();
        let mut missing: BTreeSet<String> = english.keys().cloned().collect();
        let mut imported = 0;
        let mut overrides = 0;
        let mut used = BTreeSet::new();
        for (id, msgid) in &english {
            if let Some(value) = messages.get(msgid).filter(|s| !s.is_empty()) {
                if value.contains(['\0', '\u{4}']) {
                    return Err("Cannot flatten context/plural translation".into());
                }
                strings.insert(id.clone(), value.clone());
                imported += 1;
                missing.remove(id);
                used.insert(msgid.clone());
            }
        }
        let over = root
            .join("translations-overrides")
            .join(format!("{}.toml", locale.split(['_', '-']).next().unwrap()));
        if over.exists() {
            let values: BTreeMap<String, String> = toml::from_str(&std::fs::read_to_string(over)?)?;
            for (id, value) in values {
                if !english.contains_key(&id) {
                    return Err(format!("Unknown override ID {id}").into());
                }
                missing.remove(&id);
                strings.insert(id, value);
                overrides += 1;
            }
        }
        report.push_str(&format!(
            "| {locale} | 0x{lang:04x} | {imported} | {overrides} | {} | {} |\n",
            missing.len(),
            messages.len().saturating_sub(used.len() + 1)
        ));
        details.push_str(&format!("\n## {locale}\n\nMissing IDs (English fallback): {}\n\nUnmapped original messages:\n\n",missing.into_iter().collect::<Vec<_>>().join(", ")));
        for key in messages
            .keys()
            .filter(|key| !key.is_empty() && !used.contains(*key))
        {
            details.push_str(&format!(
                "- `{}`\n",
                key.replace('`', "'")
                    .replace('\0', " [plural] ")
                    .replace('\u{4}', " [context] ")
                    .replace('\n', " ")
            ));
        }
        output.push_str(&block(lang, &strings)?);
    }
    if langs.is_empty() {
        return Err(
            "No .mo catalogues found. Compile PO files to a separate directory first.".into(),
        );
    }
    let en = format!(
        "// Generated from resources/strings.toml\n{}{}",
        block(0x0409, &english)?,
        block(0, &english)?
    );
    std::fs::write(
        root.join("resources/strings.en.rc"),
        format!("{}\n", en.trim_end()),
    )?;
    std::fs::write(
        root.join("resources/strings.generated.rc"),
        format!("{}\n", output.trim_end()),
    )?;
    report.push_str(&details);
    let report = report.replace(
        "Missing IDs (English fallback): \n",
        "Missing IDs (English fallback): none\n",
    );
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&artifacts)?;
    std::fs::write(artifacts.join("translation-report.md"), report)?;
    println!(
        "Imported {} languages into Windows STRINGTABLE resources",
        langs.len()
    );
    println!("Coverage report: artifacts/translation-report.md");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_corruption() {
        for bytes in [vec![], vec![0; 28], vec![0xde, 0x12, 0x04, 0x95]] {
            assert!(parse_mo(&bytes).is_err());
        }
    }
    #[test]
    fn handles_both_endiannesses() {
        for little in [true, false] {
            let mut bytes = Vec::new();
            for n in [0x950412deu32, 0, 1, 28, 36, 0, 0, 5, 44, 4, 50] {
                bytes.extend_from_slice(&if little {
                    n.to_le_bytes()
                } else {
                    n.to_be_bytes()
                });
            }
            bytes.extend_from_slice(b"Hello\0Ciao\0");
            assert_eq!(parse_mo(&bytes).unwrap().get("Hello").unwrap(), "Ciao");
            bytes.pop();
            assert!(parse_mo(&bytes).is_err());
        }
    }
}
