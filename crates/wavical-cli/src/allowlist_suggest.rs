//! `wavical allowlist suggest` implementation (Phase 4.5 WP6, Plan 1.7).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{BufRead, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use wavical_netpolicy::allowlist::Allowlist;
use wavical_netpolicy::origin::{SchemePolicy, parse_exact_origin};

use crate::CliError;

#[derive(Debug, Clone, Deserialize)]
struct RecordedEntry {
    #[serde(rename = "type")]
    block_type: String,
    origin: String,
    count: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct RecordedFile {
    #[serde(default)]
    entries: Vec<RecordedEntry>,
}

/// Editable JSON representation of `wavical.allowlist.json` that preserves existing entries and order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditableAllowlist {
    version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scripts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    styles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fonts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    media: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    connect: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    frames: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    navigate: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    open_external: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    open_external_schemes: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    inline_styles: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    data_images: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_ui: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    local_exceptions: Vec<serde_json::Value>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for EditableAllowlist {
    fn default() -> Self {
        Self {
            version: 1,
            scripts: Vec::new(),
            styles: Vec::new(),
            fonts: Vec::new(),
            images: Vec::new(),
            media: Vec::new(),
            connect: Vec::new(),
            frames: Vec::new(),
            navigate: Vec::new(),
            open_external: Vec::new(),
            open_external_schemes: Vec::new(),
            inline_styles: false,
            data_images: false,
            remote_ui: None,
            local_exceptions: Vec::new(),
        }
    }
}

impl EditableAllowlist {
    fn list_mut(&mut self, category: &str) -> Option<&mut Vec<String>> {
        match category {
            "scripts" => Some(&mut self.scripts),
            "styles" => Some(&mut self.styles),
            "fonts" => Some(&mut self.fonts),
            "images" => Some(&mut self.images),
            "media" => Some(&mut self.media),
            "connect" => Some(&mut self.connect),
            "frames" => Some(&mut self.frames),
            "navigate" => Some(&mut self.navigate),
            "openExternal" => Some(&mut self.open_external),
            _ => None,
        }
    }

    fn list_ref(&self, category: &str) -> Option<&[String]> {
        match category {
            "scripts" => Some(&self.scripts),
            "styles" => Some(&self.styles),
            "fonts" => Some(&self.fonts),
            "images" => Some(&self.images),
            "media" => Some(&self.media),
            "connect" => Some(&self.connect),
            "frames" => Some(&self.frames),
            "navigate" => Some(&self.navigate),
            "openExternal" => Some(&self.open_external),
            _ => None,
        }
    }
}

/// Runs `wavical allowlist suggest` using stdin/stdout.
pub fn run_allowlist_suggest(project_root: &Path) -> Result<(), CliError> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    run_allowlist_suggest_with_io(project_root, &mut reader, &mut writer)
}

/// Testable core of `wavical allowlist suggest`.
pub fn run_allowlist_suggest_with_io<R: BufRead, W: Write>(
    project_root: &Path,
    reader: &mut R,
    writer: &mut W,
) -> Result<(), CliError> {
    let recorded_path = project_root.join(".wavical").join("recorded-blocks.json");
    if !recorded_path.is_file() {
        writeln!(
            writer,
            "No recorded blocks found at {}. Run `wavical dev-safe --record` or `wavical dev-ultra-safe --record` first.",
            recorded_path.display()
        )
        .map_err(|e| CliError::ShellError(e.to_string()))?;
        return Ok(());
    }

    let raw_recorded = fs::read(&recorded_path).map_err(|e| CliError::ShellError(e.to_string()))?;
    let recorded: RecordedFile =
        serde_json::from_slice(&raw_recorded).map_err(|e| CliError::ShellError(e.to_string()))?;

    let allowlist_path = project_root.join("wavical.allowlist.json");
    let mut editable: EditableAllowlist = if allowlist_path.is_file() {
        let existing_bytes =
            fs::read(&allowlist_path).map_err(|e| CliError::ShellError(e.to_string()))?;
        Allowlist::parse_bytes(&existing_bytes).map_err(|e| {
            CliError::ShellError(format!("existing wavical.allowlist.json invalid: {e}"))
        })?;
        serde_json::from_slice(&existing_bytes).map_err(|e| CliError::ShellError(e.to_string()))?
    } else {
        EditableAllowlist::default()
    };

    let mut proposals: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    let mut needs_decision: Vec<String> = Vec::new();

    for entry in recorded.entries {
        let bt = entry.block_type.as_str();
        let orig = entry.origin.as_str();

        // Check for "needs your decision" items (never auto-proposed)
        if orig == "inline" && bt == "styles" {
            needs_decision.push(format!(
                "inline styles were blocked {} time(s); set \"inlineStyles\": true in wavical.allowlist.json if you trust your CSS sources",
                entry.count
            ));
            continue;
        }
        if orig == "inline" && bt == "scripts" {
            needs_decision.push(format!(
                "inline scripts were blocked {} time(s); move inline handlers/scripts or run `wavical dev-safe` for static script hashing",
                entry.count
            ));
            continue;
        }
        if orig == "eval" {
            needs_decision.push(format!(
                "eval / unsafe-eval was blocked {} time(s); safe mode does not permit 'unsafe-eval' (use open mode or remove eval)",
                entry.count
            ));
            continue;
        }
        if orig == "data:" && bt == "images" {
            needs_decision.push(format!(
                "data: images were blocked {} time(s); set \"dataImages\": true in wavical.allowlist.json if needed",
                entry.count
            ));
            continue;
        }
        if orig == "mailto:" || orig == "tel:" {
            let scheme = orig.trim_end_matches(':');
            needs_decision.push(format!(
                "{orig} links were blocked {} time(s); add \"{scheme}\" to \"openExternalSchemes\" in wavical.allowlist.json if trusted",
                entry.count
            ));
            continue;
        }
        if orig.contains('*') {
            needs_decision.push(format!(
                "wildcard origin '{orig}' ({bt}, {} time(s)); wildcards are never auto-proposed",
                entry.count
            ));
            continue;
        }
        if bt == "remoteUi" || bt == "bridge" || bt == "localExceptions" {
            needs_decision.push(format!(
                "{bt} '{orig}' ({} time(s)); remoteUi, bridge capabilities, and localExceptions require manual configuration",
                entry.count
            ));
            continue;
        }

        let scheme_policy = if bt == "connect" {
            SchemePolicy::HttpsOrWss
        } else {
            SchemePolicy::HttpsOnly
        };

        // Only exact non-IP https (or wss for connect) origins are proposable
        match parse_exact_origin(orig, scheme_policy, false) {
            Ok(parsed) => {
                let canonical = parsed.as_canonical();
                if let Some(existing) = editable.list_ref(bt) {
                    let already_present: HashSet<String> = existing
                        .iter()
                        .filter_map(|e| parse_exact_origin(e, scheme_policy, false).ok())
                        .map(|o| o.as_canonical())
                        .collect();
                    if !already_present.contains(&canonical) {
                        proposals
                            .entry(bt.to_string())
                            .or_default()
                            .push((canonical, entry.count));
                    }
                } else {
                    needs_decision.push(format!(
                        "{bt} '{orig}' ({} time(s)); requires manual decision",
                        entry.count
                    ));
                }
            }
            Err(_) => {
                needs_decision.push(format!(
                    "{bt} '{orig}' ({} time(s)); non-HTTPS / local / IP origin requires manual localExceptions entry with a reason if trusted",
                    entry.count
                ));
            }
        }
    }

    if !needs_decision.is_empty() {
        writeln!(writer, "\nNeeds your decision (never added automatically):")
            .map_err(|e| CliError::ShellError(e.to_string()))?;
        for msg in &needs_decision {
            writeln!(writer, "  - {msg}").map_err(|e| CliError::ShellError(e.to_string()))?;
        }
    }

    if proposals.is_empty() {
        writeln!(writer, "\nNo new allowlist origins to propose.")
            .map_err(|e| CliError::ShellError(e.to_string()))?;
        return Ok(());
    }

    writeln!(writer, "\nProposed allowlist additions:")
        .map_err(|e| CliError::ShellError(e.to_string()))?;
    for (cat, items) in &proposals {
        writeln!(writer, "  [{cat}]").map_err(|e| CliError::ShellError(e.to_string()))?;
        for (orig, count) in items {
            writeln!(writer, "    + {orig} (blocked {count} time(s))")
                .map_err(|e| CliError::ShellError(e.to_string()))?;
        }
    }

    write!(
        writer,
        "\nAccept [a]ll, [g]roup-by-group, or [n]one? [a/g/N]: "
    )
    .map_err(|e| CliError::ShellError(e.to_string()))?;
    writer
        .flush()
        .map_err(|e| CliError::ShellError(e.to_string()))?;

    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| CliError::ShellError(e.to_string()))?;
    let choice = line.trim().to_ascii_lowercase();

    let mut accepted_any = false;
    if choice == "a" || choice == "all" {
        for (cat, items) in proposals {
            if let Some(list) = editable.list_mut(&cat) {
                for (orig, _) in items {
                    if !list.contains(&orig) {
                        list.push(orig);
                        accepted_any = true;
                    }
                }
            }
        }
    } else if choice == "g" || choice == "group" {
        for (cat, items) in proposals {
            write!(writer, "Add {} origin(s) to '{cat}'? [y/N]: ", items.len())
                .map_err(|e| CliError::ShellError(e.to_string()))?;
            writer
                .flush()
                .map_err(|e| CliError::ShellError(e.to_string()))?;
            let mut ans = String::new();
            reader
                .read_line(&mut ans)
                .map_err(|e| CliError::ShellError(e.to_string()))?;
            if matches!(ans.trim().to_ascii_lowercase().as_str(), "y" | "yes")
                && let Some(list) = editable.list_mut(&cat)
            {
                for (orig, _) in items {
                    if !list.contains(&orig) {
                        list.push(orig);
                        accepted_any = true;
                    }
                }
            }
        }
    } else {
        writeln!(writer, "No changes written.").map_err(|e| CliError::ShellError(e.to_string()))?;
        return Ok(());
    }

    if !accepted_any {
        writeln!(writer, "No changes written.").map_err(|e| CliError::ShellError(e.to_string()))?;
        return Ok(());
    }

    let out_json =
        serde_json::to_string_pretty(&editable).map_err(|e| CliError::ShellError(e.to_string()))?;
    // Validate with WP2 parser BEFORE writing to disk
    Allowlist::parse_bytes(out_json.as_bytes())
        .map_err(|e| CliError::ShellError(format!("generated allowlist failed validation: {e}")))?;

    fs::write(&allowlist_path, out_json).map_err(|e| CliError::WriteFileFailed {
        path: allowlist_path.clone(),
        source: e,
    })?;
    writeln!(writer, "Updated {}.", allowlist_path.display())
        .map_err(|e| CliError::ShellError(e.to_string()))?;
    Ok(())
}
