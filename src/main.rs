mod config;
mod dates;
mod feed;
mod github;
mod json;
mod render;

use crate::config::Config;
use crate::dates::{is_rfc3339_utc, months_ago_rfc3339, now_rfc3339, now_secs};
use crate::feed::{merge_events, prune_events_before, Feed};
use crate::github::{
    candidate_report, contribution_windows, extract_contributions, fetch_comment_events,
    fetch_contributions, update_candidate_issue,
};
use crate::json::{parse_json, required_string, string_field, Json};
use crate::render::{render_bitcoin_theme_css, render_css, render_html, render_js};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, String>;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        return usage();
    };
    let rest: Vec<String> = args.collect();
    match cmd.as_str() {
        "collect" => collect_cmd(&rest),
        "render" => render_cmd(&rest),
        "validate" => validate_cmd(&rest),
        "validate-feed" => validate_feed_cmd(&rest),
        "fixture" => fixture_cmd(&rest),
        _ => usage(),
    }
}

fn usage() -> Result<()> {
    Err("usage: btc-contribs <collect|render|validate|validate-feed|fixture> [--config PATH] [--feed PATH] [--state PATH] [--out PATH]".into())
}

fn collect_cmd(args: &[String]) -> Result<()> {
    let config_path = flag(args, "--config").unwrap_or_else(|| "config/site.toml".into());
    let state_path = flag(args, "--state").unwrap_or_else(|| ".cache/feed.json".into());
    let out_path = flag(args, "--out").unwrap_or_else(|| "public/feed.json".into());
    let candidates_path =
        flag(args, "--candidates").unwrap_or_else(|| ".cache/candidates.md".into());
    let full = args.iter().any(|a| a == "--full")
        || env::var("BTC_CONTRIBS_FULL").is_ok_and(|v| v == "1" || v == "true");
    let config = Config::from_file(Path::new(&config_path))?;
    let token = env::var("GITHUB_TOKEN")
        .or_else(|_| env::var("GH_TOKEN"))
        .map_err(|_| "set GITHUB_TOKEN or GH_TOKEN for collection".to_string())?;

    let mut feed = Feed {
        generated_at: now_rfc3339(),
        username: config.username.clone(),
        events: Vec::new(),
    };
    if Path::new(&state_path).exists() && !full {
        feed = Feed::from_json_file(Path::new(&state_path))?;
        feed.generated_at = now_rfc3339();
    }

    let from = months_ago_rfc3339(config.bootstrap_months);
    let mut candidates = BTreeSet::new();
    for (window_from, window_to) in contribution_windows(config.bootstrap_months, now_secs()) {
        let graph = fetch_contributions(&token, &config.username, &window_from, &window_to)?;
        let (events, found_candidates) = extract_contributions(&config, &graph)?;
        merge_events(&mut feed.events, events);
        candidates.extend(found_candidates);
    }
    let comments = fetch_comment_events(&token, &config, &from)?;
    merge_events(&mut feed.events, comments);
    prune_events_before(&mut feed.events, &from);
    feed.events
        .sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at).then(a.id.cmp(&b.id)));

    write_parented(Path::new(&out_path), &feed.to_json())?;
    write_parented(Path::new(&state_path), &feed.to_json())?;

    if !candidates.is_empty() {
        let report = candidate_report(&config, &candidates);
        write_parented(Path::new(&candidates_path), &report)?;
        if env::var("GITHUB_REPOSITORY").is_ok() {
            update_candidate_issue(&token, &report)?;
        }
    }
    Ok(())
}

fn render_cmd(args: &[String]) -> Result<()> {
    let config_path = flag(args, "--config").unwrap_or_else(|| "config/site.toml".into());
    let feed_path = flag(args, "--feed").unwrap_or_else(|| "public/feed.json".into());
    let out_dir = flag(args, "--out").unwrap_or_else(|| "public".into());
    let config = Config::from_file(Path::new(&config_path))?;
    let feed = Feed::from_json_file(Path::new(&feed_path))?;
    let out = PathBuf::from(out_dir);
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let html = render_html(&config, &feed);
    fs::write(out.join("index.html"), &html).map_err(|e| e.to_string())?;
    fs::write(out.join("theme-bitcoin.css"), render_bitcoin_theme_css())
        .map_err(|e| e.to_string())?;
    fs::write(out.join("btc_foss.css"), render_css()).map_err(|e| e.to_string())?;
    fs::write(out.join("btc_foss.js"), render_js()).map_err(|e| e.to_string())?;
    if !out.join("feed.json").exists() {
        fs::write(out.join("feed.json"), feed.to_json()).map_err(|e| e.to_string())?;
    }
    write_local_preview_files(&out, &config, &feed, &html)?;
    Ok(())
}

fn validate_cmd(args: &[String]) -> Result<()> {
    let config_path = flag(args, "--config").unwrap_or_else(|| "config/site.toml".into());
    let config = Config::from_file(Path::new(&config_path))?;
    if config.username.is_empty() {
        return Err("config username must not be empty".into());
    }
    if !config.base_path.starts_with('/') || !config.base_path.ends_with('/') {
        return Err("base_path must start and end with '/'".into());
    }
    Ok(())
}

fn validate_feed_cmd(args: &[String]) -> Result<()> {
    let feed_path = flag(args, "--feed").unwrap_or_else(|| "public/feed.json".into());
    let raw = fs::read_to_string(&feed_path).map_err(|e| format!("{feed_path}: {e}"))?;
    let json = parse_json(&raw)?;
    required_string(&json, &["generated_at"])?;
    required_string(&json, &["username"])?;
    let events = json
        .get("events")
        .and_then(Json::array)
        .ok_or_else(|| "feed events must be an array".to_string())?;

    for (index, event) in events.iter().enumerate() {
        for key in [
            "id",
            "event_type",
            "repo",
            "title",
            "url",
            "occurred_at",
            "thread_id",
            "thread_title",
            "thread_url",
        ] {
            required_string(event, &[key]).map_err(|err| format!("event {index}: {err}"))?;
        }
        string_field(event, &["status"]).map_err(|err| format!("event {index}: {err}"))?;
        let event_type = event
            .get("event_type")
            .and_then(Json::string)
            .unwrap_or_default();
        if !matches!(
            event_type,
            "pull_request" | "issue" | "commit" | "review" | "comment"
        ) {
            return Err(format!(
                "event {index}: unsupported event_type {event_type}"
            ));
        }
        let url = event.get("url").and_then(Json::string).unwrap_or_default();
        if !url.starts_with("https://github.com/") {
            return Err(format!("event {index}: url must point to github.com"));
        }
        let occurred_at = event
            .get("occurred_at")
            .and_then(Json::string)
            .unwrap_or_default();
        if !is_rfc3339_utc(occurred_at) {
            return Err(format!(
                "event {index}: occurred_at must be RFC3339-like UTC"
            ));
        }
    }

    Ok(())
}

fn write_local_preview_files(out: &Path, config: &Config, feed: &Feed, html: &str) -> Result<()> {
    let base_dir_name = config.base_path.trim_matches('/');
    if !base_dir_name.is_empty() {
        let base_dir = out.join(base_dir_name);
        fs::create_dir_all(&base_dir).map_err(|e| e.to_string())?;
        fs::write(base_dir.join("index.html"), html).map_err(|e| e.to_string())?;
        fs::write(
            base_dir.join("theme-bitcoin.css"),
            render_bitcoin_theme_css(),
        )
        .map_err(|e| e.to_string())?;
        fs::write(base_dir.join("btc_foss.css"), render_css()).map_err(|e| e.to_string())?;
        fs::write(base_dir.join("btc_foss.js"), render_js()).map_err(|e| e.to_string())?;
        fs::write(base_dir.join("feed.json"), feed.to_json()).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn fixture_cmd(args: &[String]) -> Result<()> {
    let config_path = flag(args, "--config").unwrap_or_else(|| "config/site.toml".into());
    let feed_path = flag(args, "--feed").unwrap_or_else(|| "fixtures/feed.json".into());
    let out_dir = flag(args, "--out").unwrap_or_else(|| "public".into());
    render_cmd(&[
        "--config".into(),
        config_path,
        "--feed".into(),
        feed_path,
        "--out".into(),
        out_dir,
    ])
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find_map(|w| (w[0] == name).then(|| w[1].clone()))
}

fn write_parented(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, body).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_feed_fixture() {
        validate_feed_cmd(&["--feed".into(), "fixtures/feed.json".into()]).unwrap();
    }
}
