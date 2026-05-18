# TODO

External-user release sweep (#56 + #57) shipped on 2026-05-18: repo description, topics, GitHub Releases backfilled, README lede + quick-start, GHCR multi-arch image pipeline, standalone `docker-compose.example.yml`. The release-intent doctrine — _"don't sharpen for optics; build for the 4 CCs"_ — still applies: no further external-polish work without actual external signal (someone files an issue, asks for help, or otherwise shows up).

## Open

_(empty)_

## Closed (2026-05-18 release sweep)

- ~~Tier 1: repo description + topics, GitHub Releases backfill for v1.7.0/v1.8.0/v3.2.0, README lede + quick-start.~~ Shipped in #56.
- ~~Tier 2: GHCR multi-arch release pipeline, standalone `docker-compose.example.yml`, README quick-start refresh.~~ Shipped in #57; `v3.2.0` image live and public at `ghcr.io/thek3nsai/ops-brain`.
- ~~Node 20 → Node 24 action versions across `.github/workflows/`.~~ Bumped to explicit Node-24 majors ahead of GitHub's 2026-06-02 auto-force: checkout v5→v6, upload-artifact v4→v7, download-artifact v4→v8, setup-buildx-action v3→v4, login-action v3→v4, build-push-action v6→v7, action-gh-release v2→v3. `actions/cache@v5` already on Node 24.
