# btc_foss

Static Bitcoin/FOSS proof-of-work feed for public GitHub contribution activity.

This repository builds a Rust collector/renderer that publishes `feed.json` through GitHub Pages. A Vercel React site can fetch that JSON directly instead of requiring generated data to be committed back to the repo.

Expected feed URL:

```text
https://noahjoeris.github.io/btc_foss/feed.json
```

## How It Works

The GitHub Actions workflow in `.github/workflows/pages.yml` runs on push, daily cron, monthly full recrawl, and manual dispatch.

It:

1. Builds the Rust binary.
2. Collects GitHub activity into `public/feed.json`.
3. Renders the static page into `public/`.
4. Deploys `public/` to GitHub Pages.

Generated files under `public/` are ignored and should not be edited directly.

## Configuration

Edit `config/site.toml`:

- `username`: GitHub user to collect activity for.
- `site_root`: GitHub Pages origin.
- `base_path`: Pages path, currently `/btc_foss/`.
- `bootstrap_months`: contribution window, currently 24 months.
- `allowlist`: repositories that may appear in the public feed.

## GitHub Setup

Enable GitHub Pages for this repository:

- Source: GitHub Actions

Optional secret:

```text
BTC_CONTRIBS_TOKEN
```

A fine-grained PAT with read access to public repositories is enough for public activity. The workflow falls back to `github.token`, but a PAT can avoid rate-limit and permission quirks.

Trigger the workflow manually with `full=true` for the first full collection.

## Vercel Usage

```ts
const res = await fetch("https://noahjoeris.github.io/btc_foss/feed.json", {
  next: { revalidate: 3600 },
});

const feed = await res.json();
```

## Local Verification

```sh
cargo fmt -- --check
cargo test
cargo clippy -- -D warnings
cargo run -- validate --config config/site.toml
cargo run -- fixture --config config/site.toml --feed fixtures/feed.json --out public
```

Preview locally:

```sh
python3 -m http.server 8081 --directory public
```

Then open:

```text
http://127.0.0.1:8081/btc_foss/
```

## Feed Shape

```json
{
  "generated_at": "...",
  "username": "noahjoeris",
  "events": [
    {
      "id": "...",
      "event_type": "pull_request",
      "repo": "owner/repo",
      "title": "...",
      "url": "...",
      "occurred_at": "...",
      "thread_id": "...",
      "thread_title": "...",
      "thread_url": "...",
      "status": "..."
    }
  ]
}
```
