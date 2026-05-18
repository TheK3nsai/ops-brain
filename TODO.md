# TODO

External-user release sweep (#56 + #57) shipped on 2026-05-18: repo description, topics, GitHub Releases backfilled, README lede + quick-start, GHCR multi-arch image pipeline, standalone `docker-compose.example.yml`. The release-intent doctrine — _"don't sharpen for optics; build for the 4 CCs"_ — still applies: no further external-polish work without actual external signal (someone files an issue, asks for help, or otherwise shows up).

## Open

- **Node 20 → Node 24 action versions in `.github/workflows/release.yml`.** GitHub auto-forces Node 24 on **2026-06-02** and removes Node 20 on **2026-09-16**. The `@v4`/`@v6`/`@v3` pins will get auto-upgraded; bump them to the maintainers' explicit Node-24 releases when those land so the deprecation annotation goes away.

## Closed (2026-05-18 release sweep)

- ~~Tier 1: repo description + topics, GitHub Releases backfill for v1.7.0/v1.8.0/v3.2.0, README lede + quick-start.~~ Shipped in #56.
- ~~Tier 2: GHCR multi-arch release pipeline, standalone `docker-compose.example.yml`, README quick-start refresh.~~ Shipped in #57; `v3.2.0` image live and public at `ghcr.io/thek3nsai/ops-brain`.
