use crate::config::Config;
use crate::dates::short_date;
use crate::feed::{Event, Feed};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

pub fn render_html(config: &Config, feed: &Feed) -> String {
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

pub fn render_bitcoin_theme_css() -> &'static str {
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

pub fn render_css() -> &'static str {
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

pub fn render_js() -> &'static str {
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

fn html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html_attr(s: &str) -> String {
    html(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
