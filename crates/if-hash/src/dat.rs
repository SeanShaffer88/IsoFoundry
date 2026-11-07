//! Logiqx XML `.dat` files (the format redump.org and ClrMamePro use):
//! write one for images IsoFoundry makes, and read known-good lists to
//! verify images against.

use crate::Digests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rom {
    pub name: String,
    pub size: u64,
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
}

impl Rom {
    pub fn from_digests(name: &str, d: &Digests) -> Self {
        Rom {
            name: name.to_string(),
            size: d.size,
            crc32: d.crc32.clone(),
            md5: d.md5.clone(),
            sha1: d.sha1.clone(),
            sha256: d.sha256.clone(),
        }
    }

    pub fn digests(&self) -> Digests {
        Digests {
            size: self.size,
            crc32: self.crc32.clone(),
            md5: self.md5.clone(),
            sha1: self.sha1.clone(),
            sha256: self.sha256.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub name: String,
    pub roms: Vec<Rom>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dat {
    pub name: String,
    pub description: String,
    pub games: Vec<Game>,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(j) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let ent = &tail[1..j];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=j]),
        }
        rest = &tail[j + 1..];
    }
    out.push_str(rest);
    out
}

impl Dat {
    pub fn to_xml(&self) -> String {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<datafile>\n\t<header>\n");
        s += &format!(
            "\t\t<name>{}</name>\n\t\t<description>{}</description>\n",
            esc(&self.name),
            esc(&self.description)
        );
        s += &format!("\t\t<author>IsoFoundry {}</author>\n\t</header>\n", env!("CARGO_PKG_VERSION"));
        for g in &self.games {
            s += &format!("\t<game name=\"{}\">\n\t\t<description>{}</description>\n", esc(&g.name), esc(&g.name));
            for r in &g.roms {
                s += &format!("\t\t<rom name=\"{}\" size=\"{}\"", esc(&r.name), r.size);
                for (k, v) in [("crc", &r.crc32), ("md5", &r.md5), ("sha1", &r.sha1), ("sha256", &r.sha256)] {
                    if let Some(v) = v {
                        s += &format!(" {k}=\"{v}\"");
                    }
                }
                s += "/>\n";
            }
            s += "\t</game>\n";
        }
        s += "</datafile>\n";
        s
    }

    /// Lenient reader: takes `<game>`/`<machine>` elements and their
    /// `<rom>` attributes, ignores everything else. Input is untrusted; the
    /// parser never allocates more than the input size.
    pub fn parse(xml: &str) -> Result<Dat, String> {
        let mut dat = Dat::default();
        if let Some(h) = between(xml, "<header>", "</header>") {
            dat.name = between(h, "<name>", "</name>").map(unesc).unwrap_or_default();
            dat.description = between(h, "<description>", "</description>").map(unesc).unwrap_or_default();
        }
        let mut rest = xml;
        while let Some(pos) = find_tag(rest, &["<game", "<machine"]) {
            let tail = &rest[pos..];
            let open_end = tail.find('>').ok_or("unterminated <game> tag")?;
            let open = &tail[..open_end];
            let name = attr(open, "name").unwrap_or_default();
            let (body, consumed) = if open.ends_with('/') {
                ("", open_end + 1)
            } else {
                let close = find_tag(tail, &["</game>", "</machine>"]).ok_or("missing </game>")?;
                (&tail[open_end + 1..close], close + 1)
            };
            let mut roms = Vec::new();
            let mut r = body;
            while let Some(p) = r.find("<rom") {
                let t = &r[p..];
                let e = t.find('>').ok_or("unterminated <rom> tag")?;
                let tag = &t[..e];
                let size = attr(tag, "size").and_then(|s| s.parse().ok()).ok_or("rom without a valid size")?;
                let lower = |k| attr(tag, k).map(|v| v.to_ascii_lowercase());
                roms.push(Rom {
                    name: attr(tag, "name").unwrap_or_default(),
                    size,
                    crc32: lower("crc"),
                    md5: lower("md5"),
                    sha1: lower("sha1"),
                    sha256: lower("sha256"),
                });
                r = &t[e..];
            }
            dat.games.push(Game { name, roms });
            rest = &tail[consumed..];
        }
        if dat.games.is_empty() {
            return Err("no <game> entries found; is this a Logiqx XML .dat file?".into());
        }
        Ok(dat)
    }

    /// Every (game, rom) whose hashes match `d`.
    pub fn find(&self, d: &Digests) -> Vec<(&Game, &Rom)> {
        self.games
            .iter()
            .flat_map(|g| g.roms.iter().map(move |r| (g, r)))
            .filter(|(_, r)| r.digests().matches(d))
            .collect()
    }
}

fn find_tag(s: &str, tags: &[&str]) -> Option<usize> {
    tags.iter()
        .filter_map(|t| {
            let mut from = 0;
            while let Some(p) = s[from..].find(t) {
                let at = from + p;
                // Make sure "<game" is not a prefix of a longer tag name.
                let next = s[at + t.len()..].chars().next();
                if t.ends_with('>') || matches!(next, Some(c) if c.is_whitespace() || c == '>' || c == '/') {
                    return Some(at);
                }
                from = at + t.len();
            }
            None
        })
        .min()
}

fn between<'a>(s: &'a str, a: &str, b: &str) -> Option<&'a str> {
    let i = s.find(a)? + a.len();
    let j = s[i..].find(b)? + i;
    Some(&s[i..j])
}

fn attr(tag: &str, key: &str) -> Option<String> {
    let mut from = 0;
    while let Some(p) = tag[from..].find(key) {
        let at = from + p;
        let before_ok = at == 0 || tag.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[at + key.len()..].trim_start();
        if before_ok {
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim_start();
                let q = rest.chars().next()?;
                if q == '"' || q == '\'' {
                    let end = rest[1..].find(q)?;
                    return Some(unesc(&rest[1..1 + end]));
                }
            }
        }
        from = at + key.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let d = Digests {
            size: 3,
            crc32: Some("352441c2".into()),
            md5: Some("900150983cd24fb0d6963f7d28e17f72".into()),
            sha1: None,
            sha256: None,
        };
        let dat = Dat {
            name: "Test & <set>".into(),
            description: "d".into(),
            games: vec![Game { name: "Disc \"A\"".into(), roms: vec![Rom::from_digests("a.iso", &d)] }],
        };
        let back = Dat::parse(&dat.to_xml()).unwrap();
        assert_eq!(back, Dat { description: "d".into(), ..dat.clone() });
        assert_eq!(back.find(&d).len(), 1);
    }

    #[test]
    fn reads_redump_style() {
        let xml = r#"<?xml version="1.0"?>
<!DOCTYPE datafile PUBLIC "-//Logiqx//DTD ROM Management Datafile//EN" "http://www.logiqx.com/Dats/datafile.dtd">
<datafile>
	<header><name>Sony - PlayStation</name><description>x</description></header>
	<game name="Game (USA)">
		<category>Games</category>
		<description>Game (USA)</description>
		<rom name="Game (USA).cue" size="85" crc="C3B9F2E1" md5="0123" sha1="ABCD"/>
		<rom name="Game (USA).bin" size="12345" crc="DEADBEEF" md5="4567" sha1="EF01"/>
	</game>
	<gameplus name="not-a-game"/>
	<machine name="M"><rom name="m.bin" size="1" crc="00000001"/></machine>
</datafile>"#;
        let d = Dat::parse(xml).unwrap();
        assert_eq!(d.name, "Sony - PlayStation");
        assert_eq!(d.games.len(), 2);
        assert_eq!(d.games[0].roms[1].crc32.as_deref(), Some("deadbeef"));
        let probe = Digests { size: 12345, crc32: Some("DEADBEEF".into()), ..Default::default() };
        assert_eq!(d.find(&probe)[0].0.name, "Game (USA)");
    }

    #[test]
    fn rejects_garbage() {
        assert!(Dat::parse("hello").is_err());
        assert!(Dat::parse("<datafile><game name=\"x\"><rom name=\"a\"/></game></datafile>").is_err());
    }
}
