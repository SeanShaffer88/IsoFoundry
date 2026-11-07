//! Name rules for each file system: ISO 9660 (level 2 style d-characters),
//! Joliet (64 UCS-2 units) and UDF (127 UCS-2 units), all made unique
//! within a folder.

use std::collections::HashSet;

fn split_ext(name: &str, dir: bool) -> (&str, Option<&str>) {
    if dir {
        return (name, None);
    }
    match name.rfind('.') {
        Some(p) if p > 0 => (&name[..p], Some(&name[p + 1..])),
        _ => (name, None),
    }
}

fn unique(seen: &mut HashSet<String>, make: impl Fn(Option<u32>) -> String) -> String {
    let first = make(None);
    if seen.insert(first.to_ascii_uppercase()) {
        return first;
    }
    for n in 1.. {
        let c = make(Some(n));
        if seen.insert(c.to_ascii_uppercase()) {
            return c;
        }
    }
    unreachable!()
}

fn shorten(base: &str, max: usize, n: Option<u32>) -> String {
    let suffix = n.map(|n| format!("~{n}")).unwrap_or_default();
    let keep = max.saturating_sub(suffix.len());
    let mut b: String = base.chars().take(keep).collect();
    b.push_str(&suffix);
    b
}

/// ISO 9660 identifier (without the ";1" version), uppercase d-characters,
/// at most 30 characters plus version for files and 31 for folders.
pub fn iso(name: &str, dir: bool, seen: &mut HashSet<String>) -> String {
    let map = |s: &str| -> String {
        s.chars()
            .map(|c| c.to_ascii_uppercase())
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
            .collect()
    };
    let (base, ext) = split_ext(name, dir);
    let base = map(base);
    let ext = ext.map(|e| map(e).chars().take(8).collect::<String>());
    let base = if base.is_empty() { "_".to_string() } else { base };
    unique(seen, |n| match &ext {
        Some(e) if !e.is_empty() => format!("{}.{e}", shorten(&base, 29 - e.len(), n)),
        _ if dir => shorten(&base, 31, n),
        _ => shorten(&base, 29, n),
    })
}

fn unicode(name: &str, dir: bool, max_units: usize, bad: &[char], seen: &mut HashSet<String>) -> String {
    let clean: String = name.chars().map(|c| if c < ' ' || bad.contains(&c) { '_' } else { c }).collect();
    let (base, ext) = split_ext(&clean, dir);
    let units = |s: &str| s.encode_utf16().count();
    let fit = |b: &str, suffix: &str, ext: Option<&str>| -> String {
        let ext_part = ext.map(|e| format!(".{e}")).unwrap_or_default();
        let budget = max_units.saturating_sub(units(suffix) + units(&ext_part));
        let mut out = String::new();
        for c in b.chars() {
            if units(&out) + c.len_utf16() > budget {
                break;
            }
            out.push(c);
        }
        format!("{out}{suffix}{ext_part}")
    };
    let ext = ext.filter(|e| units(e) + 2 < max_units);
    unique(seen, |n| fit(base, &n.map(|n| format!("~{n}")).unwrap_or_default(), ext))
}

/// Joliet: up to 64 UCS-2 units; `* / : ; ? \` are not allowed.
pub fn joliet(name: &str, dir: bool, seen: &mut HashSet<String>) -> String {
    unicode(name, dir, 64, &['*', '/', ':', ';', '?', '\\'], seen)
}

/// UDF: up to 127 UCS-2 units (255 bytes with the compression byte).
pub fn udf(name: &str, dir: bool, seen: &mut HashSet<String>) -> String {
    unicode(name, dir, 127, &['/', '\0'], seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_names() {
        let mut s = HashSet::new();
        assert_eq!(iso("readme.txt", false, &mut s), "README.TXT");
        assert_eq!(iso("ReadMe.txt", false, &mut s), "README~1.TXT");
        assert_eq!(iso("my folder", true, &mut s), "MY_FOLDER");
        let long = iso(&"a".repeat(50), false, &mut s);
        assert!(long.len() <= 30);
        assert_eq!(iso("Ünï.dat", false, &mut s), "_N_.DAT");
    }

    #[test]
    fn joliet_names() {
        let mut s = HashSet::new();
        let n = joliet(&format!("{}.txt", "x".repeat(100)), false, &mut s);
        assert_eq!(n.encode_utf16().count(), 64);
        assert!(n.ends_with(".txt"));
        let n2 = joliet(&format!("{}.txt", "x".repeat(100)), false, &mut s);
        assert!(n2.ends_with("~1.txt"), "{n2}");
        assert_eq!(joliet("a:b?.c", false, &mut s), "a_b_.c");
    }

    #[test]
    fn udf_names_keep_unicode() {
        let mut s = HashSet::new();
        assert_eq!(udf("Ünïcödé – x.txt", false, &mut s), "Ünïcödé – x.txt");
    }
}
