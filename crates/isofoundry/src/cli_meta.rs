//! Meta commands (M6): `completions` (shell completion scripts),
//! `check-update` (opt-in online version check) and
//! `explorer-verbs` (per-user Explorer right-click entries).

use crate::cli::{Global, Out};
use if_core::error::Exit;
use serde_json::json;

/// Every command name the CLI accepts (tasks come from the grid,
/// so this list cannot drift from `list`).
pub fn command_names() -> Vec<String> {
    let mut names: Vec<String> = [
        "list",
        "helper-check",
        "report",
        "clean",
        "help",
        "version",
        "completions",
        "check-update",
        "explorer-verbs",
        "usb",
        "windows-to-usb",
        "mount",
        "unmount",
        "extract",
        "compare",
        "verify",
        "info",
        "erase",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for t in if_core::tasks::all() {
        names.push(t.cli.to_string());
        for v in t.variants {
            names.push(v.id.to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}

pub fn powershell_script() -> String {
    let names = command_names().join("','");
    format!(
        "# IsoFoundry command completion for PowerShell.\n\
         # Add this to your $PROFILE to load it in every session.\n\
         Register-ArgumentCompleter -CommandName isofoundry,isofoundry.com -ScriptBlock {{\n\
         \x20   param($wordToComplete, $commandAst, $cursorPosition)\n\
         \x20   if ($commandAst.CommandElements.Count -le 2) {{\n\
         \x20       @('{names}') | Where-Object {{ $_ -like \"$wordToComplete*\" }} |\n\
         \x20           ForEach-Object {{ [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }}\n\
         \x20   }}\n\
         }}\n"
    )
}

pub fn bash_script() -> String {
    let names = command_names().join(" ");
    format!(
        "# IsoFoundry command completion for bash.\n\
         # Source this file (or add it to ~/.bashrc).\n\
         _isofoundry() {{\n\
         \x20   if [ \"$COMP_CWORD\" -eq 1 ]; then\n\
         \x20       COMPREPLY=( $(compgen -W \"{names}\" -- \"${{COMP_WORDS[COMP_CWORD]}}\") )\n\
         \x20   fi\n\
         }}\n\
         complete -F _isofoundry isofoundry isofoundry.com\n"
    )
}

pub(crate) fn run_completions(tail: &[String], _g: &Global, o: &mut Out) -> i32 {
    let shell = tail.first().map(String::as_str).unwrap_or("powershell");
    match shell {
        "powershell" => {
            o.ok_text(&powershell_script());
            0
        }
        "bash" => {
            o.ok_text(&bash_script());
            0
        }
        other => o.fail(Exit::Usage, &format!("unknown shell {other} (powershell, bash)")),
    }
}

/// The `tag_name` of a GitHub release JSON document.
pub fn parse_latest_tag(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("tag_name")?.as_str().map(|s| s.to_string())
}

fn version_parts(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split('.')
        .map(|p| {
            let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().unwrap_or(0)
        })
        .collect()
}

/// True when `latest` (e.g. "v0.6.0") is newer than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (a, b) = (version_parts(latest), version_parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

pub fn user_agent() -> String {
    format!("IsoFoundry/{} (+https://github.com/SeanShaffer88/IsoFoundry)", if_core::APP_VERSION)
}

/// The newest release tag from GitHub, via the Windows HTTP helper.
/// Used by the CLI command and by the window's opt-in start check.
pub fn fetch_latest_tag() -> Result<String, String> {
    #[cfg(windows)]
    {
        let body = crate::win::http::get(
            "https://api.github.com/repos/SeanShaffer88/IsoFoundry/releases/latest",
            &user_agent(),
        )?;
        parse_latest_tag(&body).ok_or_else(|| "the release answer had no tag".to_string())
    }
    #[cfg(not(windows))]
    {
        Err("online checks need Windows".into())
    }
}

pub(crate) fn run_check_update(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    if let Some(x) = tail.first() {
        return o.fail(Exit::Usage, &format!("unexpected argument: {x}"));
    }
    match fetch_latest_tag() {
        Ok(tag) => {
            let newer = is_newer(&tag, if_core::APP_VERSION);
            if g.json {
                o.ok_text(
                    &json!({ "ok": true, "current": if_core::APP_VERSION,
                              "latest": tag, "update_available": newer })
                    .to_string(),
                );
            } else if newer {
                o.ok_text(&format!(
                    "IsoFoundry {} is available (you run {}). Download it from https://github.com/SeanShaffer88/IsoFoundry/releases",
                    tag.trim_start_matches('v'),
                    if_core::APP_VERSION
                ));
            } else {
                o.ok_text(&format!("IsoFoundry {} is the newest release.", if_core::APP_VERSION));
            }
            0
        }
        Err(m) => o.fail(Exit::Generic, &format!("update check failed: {m}")),
    }
}

/// One Explorer verb: file extension, registry verb name, menu
/// label, and the command line Explorer runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Verb {
    pub ext: String,
    pub name: String,
    pub label: String,
    pub command: String,
}

pub fn verb_definitions(exe_dir: &std::path::Path) -> Vec<Verb> {
    let cli = exe_dir.join("isofoundry.com").to_string_lossy().into_owned();
    let mut out = Vec::new();
    for ext in [".iso", ".img", ".bin", ".cue", ".ccd", ".chd"] {
        out.push(Verb {
            ext: ext.into(),
            name: "IsoFoundryChecksum".into(),
            label: "Checksum with IsoFoundry".into(),
            command: format!("cmd.exe /k \"\"{cli}\" checksum \"%1\"\""),
        });
        out.push(Verb {
            ext: ext.into(),
            name: "IsoFoundryInfo".into(),
            label: "Image info with IsoFoundry".into(),
            command: format!("cmd.exe /k \"\"{cli}\" image-info \"%1\"\""),
        });
    }
    out
}

pub(crate) fn run_explorer_verbs(tail: &[String], _g: &Global, o: &mut Out) -> i32 {
    let sub = tail.first().map(String::as_str).unwrap_or("status");
    #[cfg(windows)]
    {
        match crate::win::verbs::run(sub) {
            Ok(msg) => {
                o.ok_text(&msg);
                0
            }
            Err(m) => o.fail(Exit::Generic, &m),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = sub;
        o.fail(Exit::Generic, "Explorer verbs need Windows")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_name_real_commands() {
        let ps = powershell_script();
        assert!(ps.contains("backup-restore"));
        assert!(ps.contains("Register-ArgumentCompleter"));
        let sh = bash_script();
        assert!(sh.contains("copy-to-image"));
        assert!(sh.contains("complete -F _isofoundry"));
    }

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.6.0", "0.5.0"));
        assert!(is_newer("0.5.1", "0.5.0"));
        assert!(!is_newer("v0.5.0", "0.5.0"));
        assert!(!is_newer("0.4.9", "0.5.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
    }

    #[test]
    fn tag_parse() {
        assert_eq!(parse_latest_tag(r#"{"tag_name":"v0.6.0","name":"x"}"#).unwrap(), "v0.6.0");
        assert!(parse_latest_tag("{}").is_none());
        assert!(parse_latest_tag("not json").is_none());
    }

    #[test]
    fn verbs_cover_images() {
        let verbs = verb_definitions(std::path::Path::new(r"C:\Apps\IsoFoundry"));
        assert_eq!(verbs.len(), 12);
        assert!(verbs.iter().all(|v| v.command.contains("isofoundry.com")));
        assert!(verbs.iter().any(|v| v.ext == ".iso" && v.label.contains("Checksum")));
    }
}
