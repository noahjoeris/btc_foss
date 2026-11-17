use crate::config::{repo_allowed, repo_owner_allowed, Config};
use crate::dates::{date_days, date_from_days, months_before_rfc3339, now_rfc3339};
use crate::feed::Event;
use crate::json::{json_escape, parse_json, Json};
use crate::Result;
use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::process::Command;

pub fn fetch_contributions(token: &str, login: &str, from: &str, to: &str) -> Result<Json> {
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

pub fn extract_contributions(
    config: &Config,
    graph: &Json,
) -> Result<(Vec<Event>, BTreeSet<String>)> {
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

pub fn fetch_comment_events(token: &str, config: &Config, from: &str) -> Result<Vec<Event>> {
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

pub fn contribution_windows(months: i64, to_secs: i64) -> Vec<(String, String)> {
    let months = months.max(1);
    let step = 6;
    let mut windows = Vec::new();
    let mut end_offset = 0;
    while end_offset < months {
        let start_offset = (end_offset + step).min(months);
        windows.push((
            months_before_rfc3339(to_secs, start_offset),
            months_before_rfc3339(to_secs, end_offset),
        ));
        end_offset = start_offset;
    }
    windows
}

pub fn candidate_report(config: &Config, candidates: &BTreeSet<String>) -> String {
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

pub fn update_candidate_issue(token: &str, body: &str) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::days_from_civil;

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
    fn splits_long_collection_windows() {
        let to = days_from_civil(2026, 5, 15) * 86_400;
        let windows = contribution_windows(24, to);
        assert_eq!(windows.len(), 4);
        assert_eq!(
            windows[0],
            ("2025-11-15T00:00:00Z".into(), "2026-05-15T00:00:00Z".into())
        );
        assert_eq!(windows[3].0, "2024-05-15T00:00:00Z");
        assert!(windows.iter().all(|(start, end)| start < end));
    }
}
