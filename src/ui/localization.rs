use crate::{
    platform::{module, resource, wide},
    updater::Config,
};
use windows_sys::Win32::Globalization::*;

pub struct Language {
    pub id: u16,
    pub rtl: bool,
}

fn locale_id(locale: &str) -> u16 {
    let normalized = locale.replace('_', "-");
    let mut candidate = normalized.as_str();
    loop {
        let name = match candidate {
            "en" => "en-US",
            "it" => "it-IT",
            "ar" => "ar-SA",
            "he" => "he-IL",
            "pt" => "pt-PT",
            "zh" => "zh-CN",
            _ => candidate,
        };
        // SAFETY: LocaleNameToLCID reads the temporary NUL-terminated locale name.
        let id = unsafe { LocaleNameToLCID(wide(name).as_ptr(), 0) } as u16;
        // Custom LCIDs such as 0x1000 carry no primary language for resource lookup.
        if id & 0x3ff != 0 {
            return id;
        }
        match candidate.rsplit_once('-') {
            Some((parent, _)) => candidate = parent,
            None => return 0x409,
        }
    }
}

impl Language {
    pub fn choose(config: &Config) -> Self {
        // SAFETY: language APIs use owned buffers and change only this UI thread.
        let id = unsafe {
            let explicit = config.lang.as_deref().map(locale_id);
            let id = explicit.or(config.langid).unwrap_or_else(|| {
                let mut count = 0;
                let mut size = 0;
                GetThreadPreferredUILanguages(
                    MUI_LANGUAGE_ID,
                    &mut count,
                    std::ptr::null_mut(),
                    &mut size,
                );
                let mut buffer = vec![0u16; size as usize];
                if size > 0
                    && GetThreadPreferredUILanguages(
                        MUI_LANGUAGE_ID,
                        &mut count,
                        buffer.as_mut_ptr(),
                        &mut size,
                    ) != 0
                {
                    let first =
                        &buffer[..buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len())];
                    u16::from_str_radix(&String::from_utf16_lossy(first), 16)
                        .unwrap_or_else(|_| GetUserDefaultUILanguage())
                } else {
                    GetUserDefaultUILanguage()
                }
            });
            let id = if id == 0 { 0x409 } else { id };
            SetThreadUILanguage(id);
            id
        };
        Self {
            id,
            rtl: [1, 13, 32, 41, 128].contains(&(id & 0x3ff)),
        }
    }
    pub fn text(&self, id: u16) -> String {
        let base = self.id & 0x3ff;
        let mut candidates = vec![self.id, base];
        // Prefer the base language's standard regional resource, rather than unrelated locales.
        let region = match base {
            0x16 => 0x0816,
            0x0a => 0x040a,
            0x1a => 0x0c1a,
            _ => 0x0400 | base,
        };
        candidates.extend([region, 0x409, 0]);
        candidates.dedup();
        for lang in candidates {
            if let Some(bytes) = resource(
                module(),
                ((id / 16 + 1) as usize) as *const u16,
                6usize as *const u16,
                Some(lang),
            ) {
                let words: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|s| u16::from_le_bytes([s[0], s[1]]))
                    .collect();
                let mut offset = 0;
                for index in 0..16 {
                    let Some(&len) = words.get(offset) else {
                        break;
                    };
                    offset += 1;
                    let Some(text) = words.get(offset..offset + len as usize) else {
                        break;
                    };
                    if index == id % 16 && !text.is_empty() {
                        return String::from_utf16_lossy(text);
                    }
                    offset += len as usize;
                }
            }
        }
        format!("[{id}]")
    }
    pub fn format(&self, id: u16, values: &[&str]) -> String {
        let mut text = self.text(id);
        for (i, value) in values.iter().enumerate() {
            text = text.replace(&format!("%{}$s", i + 1), value);
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_locales_fall_back_to_a_known_parent_or_english() {
        for (locale, expected) in [
            ("it-XX", 0x0410),
            ("it_XX", 0x0410),
            ("pt-XX", 0x0816),
            ("fr-CA", 0x0c0c),
            ("en-GB", 0x0809),
            ("zz-XX", 0x0409),
        ] {
            assert_eq!(locale_id(locale), expected, "locale {locale}");
        }
    }
}
