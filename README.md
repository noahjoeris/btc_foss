# btc_foss

Fork of `trevarj/btc_foss`, adapted into a Bitcoin/FOSS proof-of-work feed for `noahjoeris`.

It collects public GitHub activity, writes `feed.json`, renders a small static page, and deploys both through GitHub Pages.

Expected feed URL:

```text
https://noahjoeris.github.io/btc_foss/feed.json
```

## Setup

Configure [config/site.toml](config/site.toml):

- `username = "noahjoeris"`
- `site_root = "https://noahjoeris.github.io"`
- `base_path = "/btc_foss/"`
- `allowlist`: repos to include in the public feed

In GitHub:

1. Enable Pages with source `GitHub Actions`.
2. Optional: add `BTC_CONTRIBS_TOKEN` as an Actions secret.
3. Run the `btc-foss-pages` workflow manually with `full=true`.

## Vercel

```ts
const res = await fetch("https://noahjoeris.github.io/btc_foss/feed.json", {
  next: { revalidate: 3600 },
});

const feed = await res.json();
```

## Local Checks

```sh
cargo fmt -- --check
cargo test
cargo clippy -- -D warnings
cargo run -- validate --config config/site.toml
cargo run -- fixture --config config/site.toml --feed fixtures/feed.json --out public
```
