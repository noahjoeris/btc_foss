use crate::json::{json_escape, parse_json, Json};
use crate::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event {
    pub id: String,
    pub event_type: String,
    pub repo: String,
    pub title: String,
    pub url: String,
    pub occurred_at: String,
    pub thread_id: String,
    pub thread_title: String,
    pub thread_url: String,
    pub status: String,
}

#[derive(Clone, Debug, Default)]
pub struct Feed {
    pub generated_at: String,
    pub username: String,
    pub events: Vec<Event>,
}

pub fn merge_events(existing: &mut Vec<Event>, incoming: Vec<Event>) {
    let mut by_id: BTreeMap<String, Event> =
        existing.drain(..).map(|e| (e.id.clone(), e)).collect();
    for event in incoming {
        by_id.insert(event.id.clone(), event);
    }
    *existing = by_id.into_values().collect();
}

pub fn prune_events_before(events: &mut Vec<Event>, from: &str) {
    events.retain(|event| event.occurred_at.as_str() >= from);
}

impl Feed {
    pub fn from_json_file(path: &Path) -> Result<Self> {
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

    pub fn to_json(&self) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
