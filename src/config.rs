use crate::json::unescape_json_string;
use crate::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub username: String,
    pub title: String,
    pub base_path: String,
    pub site_root: String,
    pub bootstrap_months: i64,
    pub allowlist_orgs: BTreeSet<String>,
    pub allowlist: BTreeSet<String>,
    pub keywords: Vec<String>,
    pub exclude: BTreeSet<String>,
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut cfg = Config {
            title: "Bitcoin FOSS Contributions".into(),
            base_path: "/btc_foss/".into(),
            site_root: "https://noahjoeris.github.io".into(),
            bootstrap_months: 5,
            ..Config::default()
        };
        for line in logical_toml_lines(&raw) {
            let line = line.split('#').next().unwrap_or("").trim().to_string();
            if line.is_empty() || line.starts_with('[') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "username" => cfg.username = parse_toml_string(value)?,
                "title" => cfg.title = parse_toml_string(value)?,
                "base_path" => cfg.base_path = parse_toml_string(value)?,
                "site_root" => cfg.site_root = parse_toml_string(value)?,
                "bootstrap_months" => {
                    cfg.bootstrap_months = value
                        .parse()
                        .map_err(|_| "invalid bootstrap_months".to_string())?
                }
                "allowlist_orgs" => {
                    cfg.allowlist_orgs = parse_toml_array(value)?.into_iter().collect()
                }
                "allowlist" => cfg.allowlist = parse_toml_array(value)?.into_iter().collect(),
                "keywords" => cfg.keywords = parse_toml_array(value)?,
                "exclude" => cfg.exclude = parse_toml_array(value)?.into_iter().collect(),
                _ => {}
            }
        }
        if cfg.username.is_empty() {
            return Err("config/site.toml must set username".into());
        }
        Ok(cfg)
    }
}

fn logical_toml_lines(raw: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut in_array = false;
    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() && !in_array {
            continue;
        }
        if in_array {
            current.push(' ');
            current.push_str(line);
            if line.contains(']') {
                lines.push(current.trim().to_string());
                current.clear();
                in_array = false;
            }
            continue;
        }
        if line.contains('=') && line.contains('[') && !line.contains(']') {
            current.push_str(line);
            in_array = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn parse_toml_string(value: &str) -> Result<String> {
    let value = value.trim();
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        Ok(unescape_json_string(&value[1..value.len() - 1]))
    } else {
        Err(format!("expected TOML string, got {value}"))
    }
}

fn parse_toml_array(value: &str) -> Result<Vec<String>> {
    let value = value.trim();
    if value == "[]" {
        return Ok(Vec::new());
    }
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err(format!("expected TOML string array, got {value}"));
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut escape = false;
    for ch in value[1..value.len() - 1].chars() {
        if escape {
            cur.push(ch);
            escape = false;
        } else if ch == '\\' && in_str {
            escape = true;
        } else if ch == '"' {
            if in_str {
                out.push(cur.clone());
                cur.clear();
            }
            in_str = !in_str;
        } else if in_str {
            cur.push(ch);
        }
    }
    Ok(out)
}

pub fn repo_allowed(config: &Config, repo: &str) -> bool {
    if config.exclude.contains(repo) {
        return false;
    }
    config.allowlist.contains(repo) || repo_owner_allowed(config, repo)
}

pub fn repo_owner_allowed(config: &Config, repo: &str) -> bool {
    repo.split_once('/').is_some_and(|(owner, _)| {
        config
            .allowlist_orgs
            .iter()
            .any(|org| org.eq_ignore_ascii_case(owner))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_arrays() {
        let items = parse_toml_array(r#"["bitcoin", "hwi"]"#).unwrap();
        assert_eq!(items, vec!["bitcoin", "hwi"]);
    }

    #[test]
    fn repo_allowed_supports_orgs_and_excludes() {
        let cfg = Config {
            allowlist_orgs: BTreeSet::from(["bitcoindevkit".into()]),
            allowlist: BTreeSet::from(["bitcoin/bitcoin".into()]),
            exclude: BTreeSet::from(["bitcoindevkit/noise".into()]),
            ..Config::default()
        };

        assert!(repo_allowed(&cfg, "bitcoindevkit/bdk_wallet"));
        assert!(repo_allowed(&cfg, "bitcoin/bitcoin"));
        assert!(!repo_allowed(&cfg, "bitcoindevkit/noise"));
        assert!(!repo_allowed(&cfg, "other/repo"));
    }
}
