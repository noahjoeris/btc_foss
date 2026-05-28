use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Default)]
struct Config {
    username: String,
    title: String,
    base_path: String,
    site_root: String,
    bootstrap_months: i64,
    allowlist_orgs: BTreeSet<String>,
    allowlist: BTreeSet<String>,
    keywords: Vec<String>,
    exclude: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Event {
    id: String,
    event_type: String,
    repo: String,
    title: String,
    url: String,
    occurred_at: String,
    thread_id: String,
    thread_title: String,
    thread_url: String,
    status: String,
}

#[derive(Clone, Debug, Default)]
struct Feed {
    generated_at: String,
    username: String,
    events: Vec<Event>,
}

#[derive(Clone, Debug)]
enum Json {
    Null,
    Bool(()),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

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
    let to = now_rfc3339();
    let mut candidates = BTreeSet::new();
    for (window_from, window_to) in contribution_windows(config.bootstrap_months, &to) {
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

impl Config {
    fn from_file(path: &Path) -> Result<Self> {
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

fn fetch_contributions(token: &str, login: &str, from: &str, to: &str) -> Result<Json> {
    let query = r#"
query($login:String!, $from:DateTime!, $to:DateTime!) {
  user(login:$login) {
    contributionsCollection(from:$from, to:$to) {
      pullRequestContributionsByRepository(maxRepositories:100) {
        repository { nameWithOwner description repositoryTopics(first:20) { nodes { topic { name } } } }
        contributions(first:100) { nodes { pullRequest { id number title url createdAt state closed merged } } }
      }
      issueContributionsByRepository(maxRepositories:100) {
        repository { nameWithOwner description repositoryTopics(first:20) { nodes { topic { name } } } }
        contributions(first:100) { nodes { issue { id number title url createdAt state closed } } }
      }
      commitContributionsByRepository(maxRepositories:100) {
        repository { nameWithOwner description repositoryTopics(first:20) { nodes { topic { name } } } }
        contributions(first:100) { nodes { commitCount occurredAt url } }
      }
      pullRequestReviewContributionsByRepository(maxRepositories:100) {
        repository { nameWithOwner description repositoryTopics(first:20) { nodes { topic { name } } } }
        contributions(first:100) { nodes { pullRequestReview { id url createdAt state pullRequest { number title url } } } }
      }
    }
  }
}
"#;
    let body = format!(
        r#"{{"query":"{}","variables":{{"login":"{}","from":"{}","to":"{}"}}}}"#,
        json_escape(query),
        json_escape(login),
        json_escape(from),
        json_escape(to)
    );
    let output = Command::new("curl")
        .args([
            "-sS",
            "-X",
            "POST",
            "-H",
            &format!("Authorization: bearer {token}"),
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
            "https://api.github.com/graphql",
        ])
        .output()
        .map_err(|e| format!("failed to run curl: {e}"))?;
    if !output.status.success() {
        return Err(format!("curl failed with status {}", output.status));
    }
    let text = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let json = parse_json(&text)?;
    if json.get_path(&["errors"]).is_some() {
        return Err(format!("GitHub GraphQL returned errors: {text}"));
    }
    Ok(json)
}

fn repo_allowed(config: &Config, repo: &str) -> bool {
    if config.exclude.contains(repo) {
        return false;
    }
    config.allowlist.contains(repo) || repo_owner_allowed(config, repo)
}

fn repo_owner_allowed(config: &Config, repo: &str) -> bool {
    repo.split_once('/').is_some_and(|(owner, _)| {
        config
            .allowlist_orgs
            .iter()
            .any(|org| org.eq_ignore_ascii_case(owner))
    })
}

fn extract_contributions(config: &Config, graph: &Json) -> Result<(Vec<Event>, BTreeSet<String>)> {
    let coll = graph
        .get_path(&["data", "user", "contributionsCollection"])
        .ok_or_else(|| "missing contributionsCollection in GraphQL response".to_string())?;
    let mut events = Vec::new();
    let mut candidates = BTreeSet::new();
    let groups = [
        ("pullRequestContributionsByRepository", "pull_request"),
        ("issueContributionsByRepository", "issue"),
        ("commitContributionsByRepository", "commit"),
        ("pullRequestReviewContributionsByRepository", "review"),
    ];
    for (field, kind) in groups {
        for group in coll.get(field).and_then(Json::array).unwrap_or(&[]) {
            let repo = group
                .get_path(&["repository", "nameWithOwner"])
                .and_then(Json::string)
                .unwrap_or("");
            if repo.is_empty() || config.exclude.contains(repo) {
                continue;
            }
            if !repo_allowed(config, repo)
                && repo_matches_keywords(group.get("repository"), &config.keywords)
            {
                candidates.insert(repo.to_string());
            }
            if !repo_allowed(config, repo) {
                continue;
            }
            for node in group
                .get_path(&["contributions", "nodes"])
                .and_then(Json::array)
                .unwrap_or(&[])
            {
                if let Some(event) = event_from_node(kind, repo, node, &config.username) {
                    events.push(event);
                }
            }
        }
    }
    Ok((events, candidates))
}

fn event_from_node(kind: &str, repo: &str, node: &Json, username: &str) -> Option<Event> {
    match kind {
        "pull_request" => {
            let pr = node.get("pullRequest")?;
            let id = pr.get("id")?.string()?.to_string();
            let number = pr.get("number").and_then(Json::number).unwrap_or(0.0) as i64;
            let title = pr.get("title")?.string()?.to_string();
            let url = pr.get("url")?.string()?.to_string();
            Some(Event {
                id,
                event_type: kind.into(),
                repo: repo.into(),
                title: title.clone(),
                url: url.clone(),
                occurred_at: pr.get("createdAt")?.string()?.to_string(),
                thread_id: format!("{repo}#{number}"),
                thread_title: title,
                thread_url: url,
                status: pr.get("state").and_then(Json::string).unwrap_or("").into(),
            })
        }
        "issue" => {
            let issue = node.get("issue")?;
            let id = issue.get("id")?.string()?.to_string();
            let number = issue.get("number").and_then(Json::number).unwrap_or(0.0) as i64;
            let title = issue.get("title")?.string()?.to_string();
            let url = issue.get("url")?.string()?.to_string();
            Some(Event {
                id,
                event_type: kind.into(),
                repo: repo.into(),
                title: title.clone(),
                url: url.clone(),
                occurred_at: issue.get("createdAt")?.string()?.to_string(),
                thread_id: format!("{repo}#{number}"),
                thread_title: title,
                thread_url: url,
                status: issue
                    .get("state")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .into(),
            })
        }
        "commit" => {
            let date = node.get("occurredAt")?.string()?.to_string();
            let count = node
                .get("commitCount")
                .and_then(Json::number)
                .unwrap_or(1.0) as i64;
            let url = commit_day_url(repo, username, &date);
            let title = format!(
                "{count} commit{} to {repo}",
                if count == 1 { "" } else { "s" }
            );
            Some(Event {
                id: format!("{repo}:commit:{date}:{count}"),
                event_type: kind.into(),
                repo: repo.into(),
                title: title.clone(),
                url: url.clone(),
                occurred_at: date.clone(),
                thread_id: format!("{repo}:commits:{date}"),
                thread_title: title,
                thread_url: url,
                status: String::new(),
            })
        }
        "review" => {
            let review = node.get("pullRequestReview")?;
            let pr = review.get("pullRequest")?;
            let number = pr.get("number").and_then(Json::number).unwrap_or(0.0) as i64;
            let thread_title = pr.get("title")?.string()?.to_string();
            let thread_url = pr.get("url")?.string()?.to_string();
            let status = review
                .get("state")
                .and_then(Json::string)
                .unwrap_or("")
                .to_string();
            Some(Event {
                id: review.get("id")?.string()?.to_string(),
                event_type: kind.into(),
                repo: repo.into(),
                title: format!("Reviewed {thread_title}"),
                url: review.get("url")?.string()?.to_string(),
                occurred_at: review.get("createdAt")?.string()?.to_string(),
                thread_id: format!("{repo}#{number}"),
                thread_title,
                thread_url,
                status,
            })
        }
        _ => None,
    }
}

fn commit_day_url(repo: &str, username: &str, occurred_at: &str) -> String {
    let day = occurred_at.get(0..10).unwrap_or(occurred_at);
    format!("https://github.com/{repo}/commits?author={username}&since={day}T00:00:00Z&until={day}T23:59:59Z")
}

fn fetch_comment_events(token: &str, config: &Config, from: &str) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    let start_day =
        date_days(from).ok_or_else(|| format!("invalid comment search start date: {from}"))?;
    let end_day = date_days(&now_rfc3339()).ok_or_else(|| "invalid current date".to_string())?;
    for scope in comment_search_scopes(config) {
        events.extend(fetch_comment_events_for_range(
            token, config, &scope, start_day, end_day, from,
        )?);
    }
    Ok(events)
}

fn fetch_comment_events_for_range(
    token: &str,
    config: &Config,
    scope: &str,
    start_day: i64,
    end_day: i64,
    from: &str,
) -> Result<Vec<Event>> {
    if start_day > end_day {
        return Ok(Vec::new());
    }

    let query_text = comment_search_query_text(scope, &config.username, start_day, end_day);
    let first_page = fetch_comment_search_page(token, &query_text, 1)?;
    let issue_count = first_page.issue_count;
    if issue_count >= 1000 {
        if start_day >= end_day {
            return Err(format!(
                "comment search for `{query_text}` returned {issue_count} threads in one day; narrow the scope to avoid GitHub search caps"
            ));
        }
        let mid_day = start_day + ((end_day - start_day) / 2);
        let mut events =
            fetch_comment_events_for_range(token, config, scope, start_day, mid_day, from)?;
        events.extend(fetch_comment_events_for_range(
            token,
            config,
            scope,
            mid_day + 1,
            end_day,
            from,
        )?);
        return Ok(events);
    }

    let mut events = Vec::new();
    let mut has_next = first_page.has_next;
    collect_comment_search_page(token, config, from, &first_page.threads, &mut events)?;
    let mut page = 2;
    while has_next {
        let search_page = fetch_comment_search_page(token, &query_text, page)?;
        collect_comment_search_page(token, config, from, &search_page.threads, &mut events)?;
        has_next = search_page.has_next;
        page += 1;
    }

    Ok(events)
}

fn comment_search_query_text(scope: &str, username: &str, start_day: i64, end_day: i64) -> String {
    format!(
        "{scope} commenter:{username} updated:{}..{}",
        date_from_days(start_day),
        date_from_days(end_day)
    )
}

fn collect_comment_search_page(
    token: &str,
    config: &Config,
    from: &str,
    threads: &[CommentThread],
    events: &mut Vec<Event>,
) -> Result<()> {
    for thread in threads {
        if !repo_allowed(config, &thread.repo) {
            continue;
        }
        events.extend(fetch_thread_comment_events(
            token,
            config,
            ThreadContext {
                repo: &thread.repo,
                number: thread.number,
                title: &thread.title,
                url: &thread.url,
            },
            from,
        )?);
    }
    Ok(())
}

fn comment_search_scopes(config: &Config) -> BTreeSet<String> {
    let mut scopes = BTreeSet::new();
    for org in &config.allowlist_orgs {
        scopes.insert(format!("org:{org}"));
    }
    for repo in &config.allowlist {
        if !repo_owner_allowed(config, repo) {
            scopes.insert(format!("repo:{repo}"));
        }
    }
    scopes
}

#[derive(Debug)]
struct CommentSearchPage {
    issue_count: i64,
    threads: Vec<CommentThread>,
    has_next: bool,
}

#[derive(Debug)]
struct CommentThread {
    repo: String,
    number: i64,
    title: String,
    url: String,
}

fn fetch_comment_search_page(
    token: &str,
    query_text: &str,
    page: i64,
) -> Result<CommentSearchPage> {
    let url = format!(
        "https://api.github.com/search/issues?q={}&per_page=100&page={page}",
        url_encode(query_text)
    );
    let json = curl_json(token, "GET", &url, None)?;
    parse_comment_search_page(&json, page)
}

fn parse_comment_search_page(json: &Json, page: i64) -> Result<CommentSearchPage> {
    let issue_count = json
        .get("total_count")
        .and_then(Json::number)
        .unwrap_or(0.0) as i64;
    let mut threads = Vec::new();
    for item in json.get("items").and_then(Json::array).unwrap_or(&[]) {
        let repo = item
            .get("repository_url")
            .and_then(Json::string)
            .and_then(repo_from_api_url)
            .unwrap_or("")
            .to_string();
        let number = item.get("number").and_then(Json::number).unwrap_or(0.0) as i64;
        let title = item
            .get("title")
            .and_then(Json::string)
            .unwrap_or("")
            .to_string();
        let url = item
            .get("html_url")
            .and_then(Json::string)
            .unwrap_or("")
            .to_string();
        if !repo.is_empty() && number > 0 && !title.is_empty() && !url.is_empty() {
            threads.push(CommentThread {
                repo,
                number,
                title,
                url,
            });
        }
    }
    Ok(CommentSearchPage {
        issue_count,
        threads,
        has_next: page * 100 < issue_count && page < 10,
    })
}

fn repo_from_api_url(url: &str) -> Option<&str> {
    url.strip_prefix("https://api.github.com/repos/")
}

struct ThreadContext<'a> {
    repo: &'a str,
    number: i64,
    title: &'a str,
    url: &'a str,
}

fn fetch_thread_comment_events(
    token: &str,
    config: &Config,
    thread: ThreadContext<'_>,
    from: &str,
) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    let mut page = 1;
    loop {
        let comments = fetch_thread_comments_page(token, thread.repo, thread.number, page)?;
        let comment_count = comments.len();
        for comment in &comments {
            if comment.get_path(&["user", "login"]).and_then(Json::string)
                != Some(config.username.as_str())
            {
                continue;
            }
            let occurred_at = comment
                .get("created_at")
                .and_then(Json::string)
                .unwrap_or("")
                .to_string();
            if occurred_at.is_empty() || occurred_at.as_str() < from {
                continue;
            }
            events.push(Event {
                id: json_id_string(comment.get("id")),
                event_type: "comment".into(),
                repo: thread.repo.into(),
                title: format!("Commented on {}", thread.title),
                url: comment
                    .get("html_url")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                occurred_at,
                thread_id: format!("{}#{}", thread.repo, thread.number),
                thread_title: thread.title.into(),
                thread_url: thread.url.into(),
                status: String::new(),
            });
        }
        if comment_count < 100 {
            break;
        }
        page += 1;
    }
    Ok(events)
}

fn fetch_thread_comments_page(
    token: &str,
    repo: &str,
    number: i64,
    page: i64,
) -> Result<Vec<Json>> {
    let url = format!(
        "https://api.github.com/repos/{repo}/issues/{number}/comments?per_page=100&page={page}"
    );
    let json = curl_json(token, "GET", &url, None)?;
    if let Some(comments) = json.array() {
        return Ok(comments.to_vec());
    }
    let json = curl_public_json("GET", &url, None)?;
    json.array()
        .map(<[Json]>::to_vec)
        .ok_or_else(|| format!("GitHub issue comments returned non-array response for {url}"))
}

fn json_id_string(value: Option<&Json>) -> String {
    match value {
        Some(Json::String(value)) => value.clone(),
        Some(Json::Number(value)) => format!("{value:.0}"),
        _ => String::new(),
    }
}

fn repo_matches_keywords(repo: Option<&Json>, keywords: &[String]) -> bool {
    let Some(repo) = repo else {
        return false;
    };
    let mut haystack = String::new();
    if let Some(name) = repo.get("nameWithOwner").and_then(Json::string) {
        haystack.push_str(name);
        haystack.push(' ');
    }
    if let Some(desc) = repo.get("description").and_then(Json::string) {
        haystack.push_str(desc);
        haystack.push(' ');
    }
    if let Some(topics) = repo
        .get_path(&["repositoryTopics", "nodes"])
        .and_then(Json::array)
    {
        for topic in topics {
            if let Some(name) = topic.get_path(&["topic", "name"]).and_then(Json::string) {
                haystack.push_str(name);
                haystack.push(' ');
            }
        }
    }
    let haystack = haystack.to_lowercase();
    keywords
        .iter()
        .any(|k| haystack.contains(&k.to_lowercase()))
}

fn merge_events(existing: &mut Vec<Event>, incoming: Vec<Event>) {
    let mut by_id: BTreeMap<String, Event> =
        existing.drain(..).map(|e| (e.id.clone(), e)).collect();
    for event in incoming {
        by_id.insert(event.id.clone(), event);
    }
    *existing = by_id.into_values().collect();
}

fn prune_events_before(events: &mut Vec<Event>, from: &str) {
    events.retain(|event| event.occurred_at.as_str() >= from);
}

fn contribution_windows(months: i64, to: &str) -> Vec<(String, String)> {
    let months = months.max(1);
    let step = 6;
    let mut windows = Vec::new();
    let mut end_offset = 0;
    while end_offset < months {
        let start_offset = (end_offset + step).min(months);
        let start = months_ago_rfc3339(start_offset);
        let end = if end_offset == 0 {
            to.to_string()
        } else {
            months_ago_rfc3339(end_offset)
        };
        windows.push((start, end));
        end_offset = start_offset;
    }
    windows
}

fn candidate_report(config: &Config, candidates: &BTreeSet<String>) -> String {
    let mut out = String::new();
    writeln!(out, "# Bitcoin FOSS candidate repositories\n").unwrap();
    writeln!(
        out,
        "Discovered from public GitHub activity for `{}` using Bitcoin-only keywords.",
        config.username
    )
    .unwrap();
    writeln!(
        out,
        "Curate candidates by adding approved repositories to `config/site.toml` `allowlist` or approved organizations to `allowlist_orgs`.\n"
    )
    .unwrap();
    for repo in candidates {
        writeln!(out, "- [ ] `{repo}` - https://github.com/{repo}").unwrap();
    }
    out
}

fn update_candidate_issue(token: &str, body: &str) -> Result<()> {
    let repo = env::var("GITHUB_REPOSITORY").map_err(|e| e.to_string())?;
    let title = "Bitcoin FOSS repository candidates";
    let list_url =
        format!("https://api.github.com/repos/{repo}/issues?state=open&labels=btc-foss-candidates");
    let list = curl_json(token, "GET", &list_url, None)?;
    let issue_number = list.array().and_then(|issues| {
        issues.iter().find_map(|issue| {
            (issue.get("title").and_then(Json::string) == Some(title))
                .then(|| issue.get("number").and_then(Json::number).unwrap_or(0.0) as i64)
        })
    });
    let payload = format!(
        r#"{{"title":"{}","body":"{}","labels":["btc-foss-candidates"]}}"#,
        json_escape(title),
        json_escape(body)
    );
    if let Some(number) = issue_number {
        let url = format!("https://api.github.com/repos/{repo}/issues/{number}");
        curl_json(token, "PATCH", &url, Some(&payload))?;
    } else {
        let url = format!("https://api.github.com/repos/{repo}/issues");
        curl_json(token, "POST", &url, Some(&payload))?;
    }
    Ok(())
}

fn curl_json(token: &str, method: &str, url: &str, body: Option<&str>) -> Result<Json> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "-X",
        method,
        "-H",
        &format!("Authorization: bearer {token}"),
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
    ]);
    if let Some(body) = body {
        cmd.args(["-H", "Content-Type: application/json", "-d", body]);
    }
    let output = cmd.arg(url).output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("curl {method} {url} failed: {}", output.status));
    }
    let text = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    parse_json(&text)
}

fn curl_public_json(method: &str, url: &str, body: Option<&str>) -> Result<Json> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "-X",
        method,
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
    ]);
    if let Some(body) = body {
        cmd.args(["-H", "Content-Type: application/json", "-d", body]);
    }
    let output = cmd.arg(url).output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("curl {method} {url} failed: {}", output.status));
    }
    let text = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    parse_json(&text)
}

impl Feed {
    fn from_json_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let json = parse_json(&raw)?;
        let generated_at = json
            .get("generated_at")
            .and_then(Json::string)
            .unwrap_or("")
            .to_string();
        let username = json
            .get("username")
            .and_then(Json::string)
            .unwrap_or("")
            .to_string();
        let mut events = Vec::new();
        for item in json.get("events").and_then(Json::array).unwrap_or(&[]) {
            events.push(Event {
                id: item
                    .get("id")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                event_type: item
                    .get("event_type")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                repo: item
                    .get("repo")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                title: item
                    .get("title")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                url: item
                    .get("url")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                occurred_at: item
                    .get("occurred_at")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                thread_id: item
                    .get("thread_id")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                thread_title: item
                    .get("thread_title")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                thread_url: item
                    .get("thread_url")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
                status: item
                    .get("status")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_string(),
            });
        }
        Ok(Feed {
            generated_at,
            username,
            events,
        })
    }

    fn to_json(&self) -> String {
        let mut out = String::new();
        writeln!(out, "{{").unwrap();
        writeln!(
            out,
            "  \"generated_at\": \"{}\",",
            json_escape(&self.generated_at)
        )
        .unwrap();
        writeln!(out, "  \"username\": \"{}\",", json_escape(&self.username)).unwrap();
        writeln!(out, "  \"events\": [").unwrap();
        for (i, event) in self.events.iter().enumerate() {
            if i > 0 {
                writeln!(out, ",").unwrap();
            }
            write!(
                out,
                "    {{\n      \"id\": \"{}\",\n      \"event_type\": \"{}\",\n      \"repo\": \"{}\",\n      \"title\": \"{}\",\n      \"url\": \"{}\",\n      \"occurred_at\": \"{}\",\n      \"thread_id\": \"{}\",\n      \"thread_title\": \"{}\",\n      \"thread_url\": \"{}\",\n      \"status\": \"{}\"\n    }}",
                json_escape(&event.id),
                json_escape(&event.event_type),
                json_escape(&event.repo),
                json_escape(&event.title),
                json_escape(&event.url),
                json_escape(&event.occurred_at),
                json_escape(&event.thread_id),
                json_escape(&event.thread_title),
                json_escape(&event.thread_url),
                json_escape(&event.status)
            ).unwrap();
        }
        writeln!(out, "\n  ]\n}}").unwrap();
        out
    }
}

fn render_html(config: &Config, feed: &Feed) -> String {
    let repos: BTreeSet<_> = feed.events.iter().map(|e| e.repo.as_str()).collect();
    let types: BTreeSet<_> = feed.events.iter().map(|e| e.event_type.as_str()).collect();
    let years: BTreeSet<_> = feed
        .events
        .iter()
        .filter_map(|e| e.occurred_at.get(0..4))
        .collect();
    let mut type_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for event in &feed.events {
        *type_counts.entry(&event.event_type).or_default() += 1;
    }

    let mut out = String::new();
    writeln!(out, "<!DOCTYPE html>").unwrap();
    writeln!(
        out,
        "<html lang=\"en\" data-theme=\"bitcoin\"><head><meta charset=\"UTF-8\">"
    )
    .unwrap();
    writeln!(
        out,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
    )
    .unwrap();
    writeln!(
        out,
        "<link rel=\"icon\" type=\"image/svg+xml\" href=\"https://upload.wikimedia.org/wikipedia/commons/4/46/Bitcoin.svg\">"
    )
    .unwrap();
    writeln!(
        out,
        "<link rel=\"canonical\" href=\"{}{}\">",
        html_attr(config.site_root.trim_end_matches('/')),
        html_attr(&config.base_path)
    )
    .unwrap();
    writeln!(
        out,
        "<link rel=\"stylesheet\" href=\"{}theme-bitcoin.css\">",
        html_attr(&config.base_path)
    )
    .unwrap();
    writeln!(
        out,
        "<link rel=\"stylesheet\" href=\"{}btc_foss.css\">",
        html_attr(&config.base_path)
    )
    .unwrap();
    writeln!(out, "<title>{}</title></head><body>", html(&config.title)).unwrap();
    writeln!(
        out,
        "<header><div class=\"markdown-heading\"><h1 class=\"heading-element\">{}</h1></div><p><a href=\"https://github.com/{}\">GitHub</a><a href=\"{}feed.json\">Feed JSON</a></p></header>",
        html(&config.username),
        html_attr(&config.username),
        html_attr(&config.base_path)
    )
    .unwrap();
    writeln!(out, "<main class=\"btc-page\">").unwrap();
    writeln!(
        out,
        "<h1>{}{}</h1>",
        bitcoin_logo_svg(),
        html(&config.title)
    )
    .unwrap();
    writeln!(out, "<p class=\"btc-muted\">Public GitHub activity for <a href=\"https://github.com/{0}\">{0}</a>. Updated <time datetime=\"{1}\">{1}</time>. <a href=\"{2}feed.json\">feed.json</a></p>", html_attr(&feed.username), html_attr(&feed.generated_at), html_attr(&config.base_path)).unwrap();
    writeln!(
        out,
        "<section class=\"btc-stats\" aria-label=\"Contribution summary\">"
    )
    .unwrap();
    stat(&mut out, "Events", feed.events.len());
    stat(&mut out, "Repos", repos.len());
    for (event_type, count) in &type_counts {
        stat(&mut out, event_type_count_label(event_type), *count);
    }
    writeln!(out, "</section>").unwrap();
    writeln!(out, "<form class=\"btc-filters\" id=\"btc-filters\">").unwrap();
    select(&mut out, "repo", "Repository", repos.iter().copied());
    select(&mut out, "type", "Type", types.iter().copied());
    select(&mut out, "year", "Year", years.iter().copied().rev());
    writeln!(
        out,
        "<a class=\"btc-reset\" href=\"{}\">Reset</a></form>",
        html_attr(&config.base_path)
    )
    .unwrap();
    writeln!(out, "<div class=\"btc-timeline\" id=\"btc-timeline\">").unwrap();

    let mut grouped: BTreeMap<String, Vec<&Event>> = BTreeMap::new();
    for event in &feed.events {
        grouped.entry(group_key(event)).or_default().push(event);
    }
    let mut groups: Vec<_> = grouped.into_values().collect();
    groups.sort_by(|a, b| b[0].occurred_at.cmp(&a[0].occurred_at));
    for mut group in groups {
        group.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at).then(a.id.cmp(&b.id)));
        let first = group[0];
        let year = first.occurred_at.get(0..4).unwrap_or("");
        let row_kind = group_kind(&group);
        let row_title = group_title(&group);
        writeln!(
            out,
            "<details class=\"btc-thread\" data-repo=\"{}\" data-type=\"{}\" data-year=\"{}\">",
            html_attr(&first.repo),
            html_attr(&row_kind),
            html_attr(year)
        )
        .unwrap();
        writeln!(out, "<summary><span class=\"btc-icon\">{}</span>{}<span class=\"btc-row-repo\">{}</span><time class=\"btc-row-date\" datetime=\"{}\">{}</time><span class=\"btc-row-kind\">{}</span></summary>",
            icon(&row_kind),
            row_title_cell(&group, &row_title),
            html(&first.repo),
            html_attr(&first.occurred_at),
            html(&short_date(&first.occurred_at)),
            html(&row_kind.replace('_', " "))
        ).unwrap();
        writeln!(out, "<div class=\"btc-thread-detail\">").unwrap();
        writeln!(out, "<ul>").unwrap();
        for event in group {
            writeln!(out, "<li data-type=\"{}\" data-title=\"{}\" data-url=\"{}\"><span class=\"btc-detail-icon\">{}</span><time datetime=\"{}\">{}</time> <span class=\"btc-kind\">{}</span> <a href=\"{}\">{}</a>{}</li>",
                html_attr(&event.event_type),
                html_attr(&event.title),
                html_attr(&event.url),
                icon(&event.event_type),
                html_attr(&event.occurred_at),
                html(&short_date(&event.occurred_at)),
                html(&event.event_type.replace('_', " ")),
                html_attr(&event.url),
                html(&event.title),
                status_badge(&event.status)).unwrap();
        }
        writeln!(out, "</ul></div></details>").unwrap();
    }
    writeln!(
        out,
        "</div></main><footer><p><a href=\"https://github.com/{0}\">{1}</a> - <a href=\"{2}feed.json\">feed.json</a></p></footer>",
        html_attr(&config.username),
        html(&config.username),
        html_attr(&config.base_path)
    )
    .unwrap();
    writeln!(
        out,
        "<script defer src=\"{}btc_foss.js\"></script>",
        html_attr(&config.base_path)
    )
    .unwrap();
    writeln!(out, "</body></html>").unwrap();
    out
}

fn stat(out: &mut String, label: &str, value: usize) {
    writeln!(
        out,
        "<div><strong>{value}</strong><span>{}</span></div>",
        html(&label.replace('_', " "))
    )
    .unwrap();
}

fn event_type_count_label(kind: &str) -> &'static str {
    match kind {
        "pull_request" => "pull requests",
        "review" => "reviews",
        "commit" => "commits",
        "comment" => "comments",
        "issue" => "issues",
        "mixed" => "mixed",
        _ => "events",
    }
}

fn select<'a>(out: &mut String, name: &str, label: &str, options: impl Iterator<Item = &'a str>) {
    writeln!(
        out,
        "<label>{}<select name=\"{}\"><option value=\"\">All</option>",
        html(label),
        html_attr(name)
    )
    .unwrap();
    for option in options {
        writeln!(
            out,
            "<option value=\"{}\">{}</option>",
            html_attr(option),
            html(option)
        )
        .unwrap();
    }
    writeln!(out, "</select></label>").unwrap();
}

fn status_badge(status: &str) -> String {
    if status.is_empty() {
        String::new()
    } else {
        format!(" <span class=\"btc-status\">{}</span>", html(status))
    }
}

fn group_key(event: &Event) -> String {
    format!("{}:{}", event.repo, short_date(&event.occurred_at))
}

fn group_kind(group: &[&Event]) -> String {
    let first = group[0].event_type.as_str();
    if group.iter().all(|event| event.event_type == first) {
        first.to_string()
    } else {
        "mixed".into()
    }
}

fn group_title(group: &[&Event]) -> String {
    if group.len() == 1 {
        group[0].thread_title.clone()
    } else {
        format!("{} activities", group.len())
    }
}

fn row_title_cell(group: &[&Event], title: &str) -> String {
    if group.len() != 1 {
        return format!(
            "<span class=\"btc-row-title\"><span class=\"btc-row-title-text\">{}</span></span>",
            html(title)
        );
    }

    let event = group[0];
    format!(
        "<span class=\"btc-row-title\"><a href=\"{}\">{}</a></span>",
        html_attr(&event.url),
        html(title)
    )
}

fn icon(kind: &str) -> &'static str {
    match kind {
        "pull_request" => "⑂",
        "review" => "✓",
        "commit" => "●",
        "comment" => "↩",
        "issue" => "!",
        "mixed" => "⋯",
        _ => "•",
    }
}

fn bitcoin_logo_svg() -> &'static str {
    r#"<img class="btc-logo" src="https://upload.wikimedia.org/wikipedia/commons/4/46/Bitcoin.svg" alt="Bitcoin">"#
}

fn render_bitcoin_theme_css() -> &'static str {
    r#"html[data-theme="bitcoin"],
html[data-theme="bitcoin"]::backdrop {
  color-scheme: dark;
  --bg: #0c0f14;
  --bg-deep: #05070a;
  --accent-bg: #141017;
  --panel-bg: rgba(18, 22, 29, 0.9);
  --panel-bg-strong: rgba(29, 25, 24, 0.95);
  --text: #f4e7cf;
  --text-light: #c9a875;
  --text-muted: #8b7558;
  --border: #7f5a24;
  --border-soft: rgba(247, 147, 26, 0.28);
  --accent: #f7931a;
  --accent-hover: #ffbf5f;
  --accent-text: #120900;
  --code: #ffd28a;
  --preformatted: #d8b06f;
  --marked: #f7931a;
  --disabled: #352717;
  --selection-text: #fff2db;
  --selection-bg: rgba(247, 147, 26, 0.32);
  --measure: 78rem;
  --page-gutter: clamp(0.75rem, 4vw, 4rem);
  --body-bg-start: #121922;
  --body-bg-mid: #0c0f14;
  --body-bg-end: #05070a;
  --body-ambient-top: rgba(247, 147, 26, 0.11);
  --body-ambient-bottom: rgba(55, 132, 122, 0.08);
  --body-grain-line: rgba(247, 147, 26, 0.014);
  --scanline-dark: rgba(0, 0, 0, 0.2);
  --scanline-bright: rgba(247, 147, 26, 0.026);
  --scanline-sweep-a: rgba(247, 147, 26, 0.016);
  --scanline-sweep-b: rgba(55, 132, 122, 0.008);
  --scanline-sweep-c: rgba(0, 0, 0, 0.1);
  --header-veil-top: rgba(247, 147, 26, 0.1);
  --header-veil-bottom: rgba(55, 132, 122, 0.025);
  --header-shadow: rgba(247, 147, 26, 0.09);
  --header-divider-soft: rgba(247, 147, 26, 0.22);
  --header-divider-core: rgba(55, 132, 122, 0.2);
  --main-vignette: rgba(247, 147, 26, 0.08);
  --footer-divider: rgba(247, 147, 26, 0.5);
  --heading-color: #ffd08a;
  --link-decoration: rgba(247, 147, 26, 0.68);
  --link-hover-shadow: 0 0 0.85rem rgba(247, 147, 26, 0.34);
  --link-focus-inner: 0 0 0 2px rgba(55, 132, 122, 0.32);
  --link-focus-outer: 0 0 0.95rem rgba(247, 147, 26, 0.48);
  --button-top: #ffbd59;
  --button-bottom: #f7931a;
  --button-hover-shadow: 0 0 1.15rem rgba(247, 147, 26, 0.42);
  --button-focus-shadow: 0 0 0.95rem rgba(55, 132, 122, 0.45);
  --select-overlay-top: rgba(247, 147, 26, 0.08);
  --select-overlay-bottom: rgba(55, 132, 122, 0.018);
  --select-surface: rgba(12, 15, 20, 0.68);
  --select-border: rgba(247, 147, 26, 0.64);
  --select-focus: 0 0 0 2px rgba(55, 132, 122, 0.22),
    0 0 0.7rem rgba(247, 147, 26, 0.4);
  --link-glow: 0 0 0.16rem rgba(255, 180, 80, 0.54),
    0 0 0.38rem rgba(247, 147, 26, 0.34),
    0 0 0.62rem rgba(55, 132, 122, 0.2);
  --link-glow-strong: 0 0 0.22rem rgba(255, 205, 115, 0.82),
    0 0 0.56rem rgba(247, 147, 26, 0.58),
    0 0 1.05rem rgba(55, 132, 122, 0.28);
  --crt-glow: 0 0 0.16rem rgba(247, 147, 26, 0.22),
    0 0 0.75rem rgba(55, 132, 122, 0.09);
  --crt-glow-strong: 0 0 0.22rem rgba(255, 190, 98, 0.62),
    0 0 1.05rem rgba(247, 147, 26, 0.28);
  --crt-panel-shadow: inset 0 0 0 1px rgba(247, 147, 26, 0.12),
    0 0 1.1rem rgba(55, 132, 122, 0.08);
  --grain-strength: 0.066;
  --grain-size: 112px;
  --grain-animation-speed: 2.8s;
  --grain-contrast: 310%;
  --snow-layers: none;
  --snow-size: auto;
  --snow-position: 0 0;
  --snow-position-end: 0 0;
  --snow-opacity: 0;
  --snow-animation: none;

  --btc-profit: #63d39a;
  --btc-cyan: #3a9e92;
  --btc-warning: #ffbf5f;
  --btc-danger: #d86b5f;
}
"#
}

fn render_css() -> &'static str {
    r#":root {
  --space-sm: 0.75rem;
  --space-md: 1rem;
  --space-lg: 1.5rem;
  --standard-border-radius: 0.375rem;
}
* { box-sizing: border-box; }
html { background: var(--bg-deep); color: var(--text); }
body {
  margin: 0;
  min-height: 100vh;
  color: var(--text);
  background:
    radial-gradient(circle at 20% 0%, var(--body-ambient-top), transparent 28rem),
    radial-gradient(circle at 80% 100%, var(--body-ambient-bottom), transparent 28rem),
    linear-gradient(180deg, var(--body-bg-start), var(--body-bg-mid) 45%, var(--body-bg-end));
  font: 16px/1.5 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
a { color: var(--accent-hover); text-decoration-color: var(--link-decoration); }
a:hover { color: var(--accent); text-shadow: var(--link-hover-shadow); }
header,
main,
footer {
  width: min(var(--measure), calc(100% - (2 * var(--page-gutter))));
  margin-inline: auto;
}
header {
  display: flex;
  justify-content: space-between;
  gap: var(--space-md);
  align-items: end;
  padding: var(--space-lg) 0 var(--space-sm);
  border-bottom: 1px solid var(--header-divider-soft);
}
header h1 {
  margin: 0;
  color: var(--heading-color);
  font-size: clamp(1rem, 2vw, 1.25rem);
}
header p,
footer p { margin: 0; }
header p {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-md);
}
footer {
  padding: var(--space-lg) 0;
  color: var(--text-light);
  border-top: 1px solid var(--footer-divider);
}
select {
  color: var(--text);
  background: var(--select-surface);
  border: 1px solid var(--select-border);
  border-radius: var(--standard-border-radius);
  padding: 0.35rem 0.45rem;
}
details {
  background: var(--panel-bg);
  border: 1px solid var(--border-soft);
  border-radius: var(--standard-border-radius);
  padding: 0.45rem 0.6rem;
  box-shadow: var(--crt-panel-shadow);
}
summary { cursor: pointer; }
.btc-page { padding-top: var(--space-lg); }
.btc-page h1 {
  display: flex;
  align-items: center;
  gap: 0.55rem;
}
.btc-logo {
  flex: 0 0 auto;
  inline-size: 1.85rem;
  block-size: 1.85rem;
  border: 0;
  border-radius: 50%;
  filter: drop-shadow(0 0 0.65rem rgba(247, 147, 26, 0.42));
  opacity: 1;
}
.btc-muted { color: var(--text-light); font-size: 0.92rem; }
.btc-stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(7rem, 1fr)); gap: 0.5rem; padding: 0; border: 0; background: transparent; box-shadow: none; }
.btc-stats div { border: 1px solid var(--border); border-radius: var(--standard-border-radius); padding: 0.45rem 0.55rem; background: linear-gradient(180deg, rgba(247, 147, 26, 0.07), rgba(247, 147, 26, 0.02)), var(--panel-bg); }
.btc-stats strong { display: block; color: var(--accent-hover); font-size: 1.2rem; line-height: 1; }
.btc-stats span { color: var(--text-light); font-size: 0.78rem; text-transform: uppercase; }
.btc-filters { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)) auto; gap: 0.6rem; align-items: end; margin: var(--space-lg) 0; }
.btc-filters label { font-size: 0.78rem; text-transform: uppercase; }
.btc-filters select { width: 100%; margin: 0.2rem 0 0; }
.btc-reset { align-self: center; white-space: nowrap; }
.btc-timeline { position: relative; display: grid; gap: 0.35rem; }
.btc-thread { margin: 0; border-color: var(--border-soft); }
.btc-thread[hidden] { display: none; }
.btc-thread summary { display: grid; grid-template-columns: 1.8rem minmax(10rem, 1fr) minmax(9rem, 0.65fr) 6.2rem 7.8rem; gap: 0.55rem; align-items: center; min-block-size: 2.25rem; word-break: normal; }
.btc-thread summary::marker { color: var(--accent-hover); }
.btc-row-title { display: inline-flex; align-items: center; gap: 0.3rem; min-width: 0; overflow: hidden; white-space: nowrap; color: var(--heading-color); }
.btc-row-title > :is(a, .btc-row-title-text):first-child { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.btc-row-repo { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-light); font-size: 0.78rem; font-weight: normal; }
.btc-row-date { color: var(--text-light); font-size: 0.78rem; font-weight: normal; font-variant-numeric: tabular-nums; text-align: end; white-space: nowrap; }
.btc-row-kind { display: inline-flex; justify-content: center; inline-size: 100%; color: var(--text-light); border: 1px solid var(--border-soft); border-radius: var(--standard-border-radius); padding: 0.03rem 0.32rem; font-size: 0.76rem; font-weight: normal; white-space: nowrap; text-transform: uppercase; }
.btc-icon { display: inline-grid; place-items: center; width: 1.35rem; height: 1.35rem; border: 1px solid var(--accent); border-radius: 50%; color: var(--accent-text); background: var(--accent); text-shadow: none; box-shadow: 0 0 0.65rem rgba(247, 147, 26, 0.28); }
.btc-thread-detail { margin-top: 0.45rem; padding-top: 0.45rem; border-top: 1px solid var(--border-soft); }
.btc-thread ul { margin: 0.45rem 0 0 1.75rem; padding-left: 0; list-style: none; }
.btc-thread li { display: grid; grid-template-columns: 1.25rem auto auto minmax(0, 1fr) auto; gap: 0.38rem; align-items: center; margin: 0.28rem 0; }
.btc-detail-icon { display: inline-grid; place-items: center; width: 1rem; height: 1rem; color: var(--accent-hover); }
.btc-thread time, .btc-kind, .btc-status { color: var(--text-light); font-size: 0.78rem; }
.btc-kind, .btc-status { border: 1px solid var(--border-soft); border-radius: var(--standard-border-radius); padding: 0.03rem 0.28rem; }
@media only screen and (max-width: 720px) {
  .btc-filters { grid-template-columns: 1fr; }
  .btc-thread summary { grid-template-columns: 1.5rem minmax(0, 1fr) auto; gap: 0.4rem; }
  .btc-row-repo { grid-column: 2; }
  .btc-row-kind { display: none; }
}
"#
}

fn render_js() -> &'static str {
    r#"(function () {
  const form = document.getElementById("btc-filters");
  const items = Array.from(document.querySelectorAll(".btc-thread"));
  const defaults = new Map();
  for (const item of items) {
    const summary = item.querySelector("summary");
    defaults.set(item, {
      icon: summary.querySelector(".btc-icon").textContent,
      title: summary.querySelector(".btc-row-title").innerHTML,
      kind: summary.querySelector(".btc-row-kind").textContent,
    });
  }
  if (!form) return;
  function typeLabel(type, count) {
    const labels = {
      pull_request: ["pull request", "pull requests"],
      review: ["review", "reviews"],
      commit: ["commit", "commits"],
      comment: ["comment", "comments"],
      issue: ["issue", "issues"],
    }[type] || [type.replaceAll("_", " "), `${type.replaceAll("_", " ")}s`];
    return count === 1 ? labels[0] : labels[1];
  }
  function setSummary(item, type, rows) {
    const summary = item.querySelector("summary");
    const icon = summary.querySelector(".btc-icon");
    const title = summary.querySelector(".btc-row-title");
    const kind = summary.querySelector(".btc-row-kind");
    if (!type) {
      const original = defaults.get(item);
      icon.textContent = original.icon;
      title.innerHTML = original.title;
      kind.textContent = original.kind;
      return;
    }
    icon.textContent = rows[0].querySelector(".btc-detail-icon").textContent;
    kind.textContent = type.replaceAll("_", " ");
    title.replaceChildren();
    if (rows.length === 1) {
      const link = document.createElement("a");
      link.href = rows[0].dataset.url;
      link.textContent = rows[0].dataset.title;
      title.append(link);
    } else {
      const label = document.createElement("span");
      label.className = "btc-row-title-text";
      label.textContent = `${rows.length} ${typeLabel(type, rows.length)}`;
      title.append(label);
    }
  }
  function apply() {
    const data = new FormData(form);
    const repo = data.get("repo");
    const type = data.get("type");
    const year = data.get("year");
    for (const item of items) {
      const detailRows = Array.from(item.querySelectorAll("[data-type]"));
      const matchingRows = type ? detailRows.filter((row) => row.dataset.type === type) : detailRows;
      const ok = (!repo || item.dataset.repo === repo) &&
        (!type || matchingRows.length > 0) &&
        (!year || item.dataset.year === year);
      item.hidden = !ok;
      for (const row of detailRows) {
        row.hidden = Boolean(type && row.dataset.type !== type);
      }
      if (ok) {
        setSummary(item, type, matchingRows);
      }
    }
  }
  form.addEventListener("change", apply);
})();
"#
}

fn parse_json(input: &str) -> Result<Json> {
    let mut p = Parser {
        chars: input.chars().collect(),
        pos: 0,
    };
    let value = p.value()?;
    p.ws();
    if p.pos != p.chars.len() {
        return Err("trailing JSON input".into());
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn value(&mut self) -> Result<Json> {
        self.ws();
        match self.peek() {
            Some('n') => self.literal("null", Json::Null),
            Some('t') => self.literal("true", Json::Bool(())),
            Some('f') => self.literal("false", Json::Bool(())),
            Some('"') => self.string().map(Json::String),
            Some('[') => self.array(),
            Some('{') => self.object(),
            Some('-' | '0'..='9') => self.number(),
            _ => Err("invalid JSON value".into()),
        }
    }

    fn literal(&mut self, lit: &str, value: Json) -> Result<Json> {
        for expected in lit.chars() {
            if self.bump() != Some(expected) {
                return Err(format!("expected {lit}"));
            }
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String> {
        self.expect('"')?;
        let mut out = String::new();
        while let Some(ch) = self.bump() {
            match ch {
                '"' => return Ok(out),
                '\\' => match self
                    .bump()
                    .ok_or_else(|| "unterminated escape".to_string())?
                {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{0008}'),
                    'f' => out.push('\u{000c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            code = code * 16
                                + self
                                    .bump()
                                    .and_then(|c| c.to_digit(16))
                                    .ok_or_else(|| "invalid unicode escape".to_string())?;
                        }
                        if let Some(c) = char::from_u32(code) {
                            out.push(c);
                        }
                    }
                    other => return Err(format!("invalid escape {other}")),
                },
                other => out.push(other),
            }
        }
        Err("unterminated string".into())
    }

    fn array(&mut self) -> Result<Json> {
        self.expect('[')?;
        let mut out = Vec::new();
        loop {
            self.ws();
            if self.peek() == Some(']') {
                self.bump();
                return Ok(Json::Array(out));
            }
            out.push(self.value()?);
            self.ws();
            match self.bump() {
                Some(',') => {}
                Some(']') => return Ok(Json::Array(out)),
                _ => return Err("expected ',' or ']'".into()),
            }
        }
    }

    fn object(&mut self) -> Result<Json> {
        self.expect('{')?;
        let mut out = BTreeMap::new();
        loop {
            self.ws();
            if self.peek() == Some('}') {
                self.bump();
                return Ok(Json::Object(out));
            }
            let key = self.string()?;
            self.ws();
            self.expect(':')?;
            out.insert(key, self.value()?);
            self.ws();
            match self.bump() {
                Some(',') => {}
                Some('}') => return Ok(Json::Object(out)),
                _ => return Err("expected ',' or '}'".into()),
            }
        }
    }

    fn number(&mut self) -> Result<Json> {
        let start = self.pos;
        while matches!(self.peek(), Some('-' | '+' | '.' | 'e' | 'E' | '0'..='9')) {
            self.bump();
        }
        let raw: String = self.chars[start..self.pos].iter().collect();
        Ok(Json::Number(
            raw.parse().map_err(|_| format!("invalid number {raw}"))?,
        ))
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\n' | '\r' | '\t')) {
            self.bump();
        }
    }

    fn expect(&mut self, ch: char) -> Result<()> {
        (self.bump() == Some(ch))
            .then_some(())
            .ok_or_else(|| format!("expected {ch}"))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => map.get(key),
            _ => None,
        }
    }

    fn get_path(&self, path: &[&str]) -> Option<&Json> {
        let mut cur = self;
        for key in path {
            cur = cur.get(key)?;
        }
        Some(cur)
    }

    fn string(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    fn array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(v) => Some(v),
            _ => None,
        }
    }
}

fn write_parented(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, body).map_err(|e| format!("{}: {e}", path.display()))
}

fn required_string<'a>(json: &'a Json, path: &[&str]) -> Result<&'a str> {
    let value = string_field(json, path)?;
    if value.is_empty() {
        return Err(format!("empty string field {}", path.join(".")));
    }
    Ok(value)
}

fn is_rfc3339_utc(value: &str) -> bool {
    if value.len() != 20 {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return false;
    }
    for index in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !bytes[index].is_ascii_digit() {
            return false;
        }
    }
    let month = value[5..7].parse::<u32>().unwrap_or(0);
    let day = value[8..10].parse::<u32>().unwrap_or(0);
    let hour = value[11..13].parse::<u32>().unwrap_or(24);
    let minute = value[14..16].parse::<u32>().unwrap_or(60);
    let second = value[17..19].parse::<u32>().unwrap_or(60);
    (1..=12).contains(&month) && (1..=31).contains(&day) && hour < 24 && minute < 60 && second < 60
}

fn string_field<'a>(json: &'a Json, path: &[&str]) -> Result<&'a str> {
    json.get_path(path)
        .and_then(Json::string)
        .ok_or_else(|| format!("missing string field {}", path.join(".")))
}

fn json_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            c if c.is_control() => format!("\\u{:04x}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect()
}

fn unescape_json_string(s: &str) -> String {
    s.replace("\\\"", "\"")
        .replace("\\n", "\n")
        .replace("\\\\", "\\")
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            write!(out, "%{byte:02X}").unwrap();
        }
    }
    out
}

fn html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html_attr(s: &str) -> String {
    html(s).replace('"', "&quot;")
}

fn now_rfc3339() -> String {
    unix_to_rfc3339(now_secs())
}

fn months_ago_rfc3339(months: i64) -> String {
    unix_to_rfc3339(now_secs() - months * 31 * 24 * 60 * 60)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn unix_to_rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn date_days(value: &str) -> Option<i64> {
    let date = value.get(0..10)?;
    let bytes = date.as_bytes();
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let year = date.get(0..4)?.parse::<i64>().ok()?;
    let month = date.get(5..7)?.parse::<i64>().ok()?;
    let day = date.get(8..10)?.parse::<i64>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

fn date_from_days(days: i64) -> String {
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * shifted_month + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + (m <= 2) as i64, m, d)
}

fn short_date(iso: &str) -> String {
    iso.get(0..10).unwrap_or(iso).to_string()
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

    #[test]
    fn comment_scopes_use_orgs_without_duplicate_repo_scopes() {
        let cfg = Config {
            allowlist_orgs: BTreeSet::from(["bitcoindevkit".into()]),
            allowlist: BTreeSet::from([
                "bitcoin/bitcoin".into(),
                "bitcoindevkit/bdk_wallet".into(),
            ]),
            ..Config::default()
        };
        let scopes = comment_search_scopes(&cfg);

        assert!(scopes.contains("org:bitcoindevkit"));
        assert!(scopes.contains("repo:bitcoin/bitcoin"));
        assert!(!scopes.contains("repo:bitcoindevkit/bdk_wallet"));
    }

    #[test]
    fn comment_search_uses_bounded_date_ranges() {
        let start = date_days("2026-05-01T00:00:00Z").unwrap();
        let end = date_days("2026-05-27T23:59:59Z").unwrap();

        assert_eq!(date_from_days(start), "2026-05-01");
        assert_eq!(
            comment_search_query_text("org:bitcoindevkit", "noahjoeris", start, end),
            "org:bitcoindevkit commenter:noahjoeris updated:2026-05-01..2026-05-27"
        );
    }

    #[test]
    fn extracts_org_allowlisted_contribution() {
        let cfg = Config {
            username: "noahjoeris".into(),
            allowlist_orgs: BTreeSet::from(["bitcoindevkit".into()]),
            ..Config::default()
        };
        let graph = parse_json(
            r#"{
              "data": {
                "user": {
                  "contributionsCollection": {
                    "pullRequestContributionsByRepository": [{
                      "repository": {
                        "nameWithOwner": "bitcoindevkit/bdk_wallet",
                        "description": "wallet",
                        "repositoryTopics": { "nodes": [] }
                      },
                      "contributions": {
                        "nodes": [{
                          "pullRequest": {
                            "id": "pr-1",
                            "number": 490,
                            "title": "Improve wallet signing",
                            "url": "https://github.com/bitcoindevkit/bdk_wallet/pull/490",
                            "createdAt": "2026-05-12T08:00:00Z",
                            "state": "OPEN",
                            "closed": false,
                            "merged": false
                          }
                        }]
                      }
                    }],
                    "issueContributionsByRepository": [],
                    "commitContributionsByRepository": [],
                    "pullRequestReviewContributionsByRepository": []
                  }
                }
              }
            }"#,
        )
        .unwrap();

        let (events, candidates) = extract_contributions(&cfg, &graph).unwrap();

        assert!(candidates.is_empty());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].repo, "bitcoindevkit/bdk_wallet");
    }

    #[test]
    fn merges_by_event_id() {
        let mut existing = vec![Event {
            id: "a".into(),
            event_type: "issue".into(),
            repo: "o/r".into(),
            title: "old".into(),
            url: "u".into(),
            occurred_at: "2026-01-01T00:00:00Z".into(),
            thread_id: "t".into(),
            thread_title: "t".into(),
            thread_url: "u".into(),
            status: String::new(),
        }];
        let mut replacement = existing[0].clone();
        replacement.title = "new".into();
        merge_events(&mut existing, vec![replacement]);
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].title, "new");
    }

    #[test]
    fn splits_long_collection_windows() {
        let windows = contribution_windows(24, "2026-05-15T00:00:00Z");
        assert_eq!(windows.len(), 4);
        assert_eq!(windows[0].1, "2026-05-15T00:00:00Z");
        assert!(windows.iter().all(|(start, end)| start < end));
    }

    #[test]
    fn prunes_events_before_collection_window() {
        let mut events = vec![
            Event {
                id: "old".into(),
                event_type: "comment".into(),
                repo: "bitcoin/bitcoin".into(),
                title: "old".into(),
                url: "u".into(),
                occurred_at: "2025-05-14T23:59:59Z".into(),
                thread_id: "t".into(),
                thread_title: "t".into(),
                thread_url: "u".into(),
                status: String::new(),
            },
            Event {
                id: "new".into(),
                event_type: "review".into(),
                repo: "bitcoin/bitcoin".into(),
                title: "new".into(),
                url: "u".into(),
                occurred_at: "2025-05-15T00:00:00Z".into(),
                thread_id: "t".into(),
                thread_title: "t".into(),
                thread_url: "u".into(),
                status: String::new(),
            },
        ];
        prune_events_before(&mut events, "2025-05-15T00:00:00Z");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "new");
    }

    #[test]
    fn feed_round_trip_fixture() {
        let feed = Feed::from_json_file(Path::new("fixtures/feed.json")).unwrap();
        assert!(feed.events.len() >= 3);
        assert!(feed
            .events
            .iter()
            .any(|event| event.event_type == "commit" && event.url.contains("/commits?author=")));
        let reparsed = parse_json(&feed.to_json()).unwrap();
        assert_eq!(
            reparsed.get("username").and_then(Json::string),
            Some("noahjoeris")
        );
    }

    #[test]
    fn validates_feed_fixture() {
        validate_feed_cmd(&["--feed".into(), "fixtures/feed.json".into()]).unwrap();
    }

    #[test]
    fn validates_utc_timestamp_shape() {
        assert!(is_rfc3339_utc("2026-05-27T11:10:47Z"));
        assert!(!is_rfc3339_utc("2026-99-27T11:10:47Z"));
        assert!(!is_rfc3339_utc("2026-05-27 11:10:47Z"));
    }

    #[test]
    fn renders_static_page() {
        let cfg = Config {
            username: "noahjoeris".into(),
            title: "Bitcoin FOSS Contributions".into(),
            base_path: "/btc_foss/".into(),
            site_root: "https://noahjoeris.github.io".into(),
            bootstrap_months: 24,
            allowlist_orgs: BTreeSet::from(["rust-bitcoin".into(), "bitcoindevkit".into()]),
            allowlist: BTreeSet::new(),
            keywords: vec!["bitcoin".into()],
            exclude: BTreeSet::new(),
        };
        let feed = Feed::from_json_file(Path::new("fixtures/feed.json")).unwrap();
        let html = render_html(&cfg, &feed);
        assert!(html.contains("data-theme=\"bitcoin\""));
        assert!(html.contains("/btc_foss/theme-bitcoin.css"));
        assert!(html.contains("<span>commits</span>"));
        assert!(html.contains("<span>pull requests</span>"));
        assert!(html.contains("feed.json"));
        assert!(html.contains("wizardsardine/bhwi"));
        assert!(html.contains("bitcoindevkit/bdk_wallet"));
    }
}
