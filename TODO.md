# TODO

External-user release sweep (#56 + #57) shipped Tier 1 + Tier 2: repo description, topics, GitHub Releases, README lede + quick-start, GHCR multi-arch image pipeline, standalone `docker-compose.example.yml`. What's left is graded by "would this actually move the needle for an external user" — nothing here is load-bearing for current operators; everything ships only if external interest materializes.

## Tier 3 — external discoverability (do only if traffic shows up)

- **`CONTRIBUTING.md`** — short, hard-points-at-`ROADMAP.md` for the kill list. The "we don't build X" doctrine is genuinely interesting positioning; surface it instead of burying it in the roadmap. Keep it terse: how to run tests locally, pointer to `/prereview` for non-trivial changes, the env-var/compose-drift checklist already in the reviewer agent.
- **Submit to [punkpeye/awesome-mcp-servers](https://github.com/punkpeye/awesome-mcp-servers)** and the [official MCP server registry](https://github.com/modelcontextprotocol/servers) once we have any external user feedback to cite. Submitting cold (0 stars, 0 users) is a worse pitch than waiting until the second installer.
- **crates.io publish** — `cargo install ops-brain` works as an alternate path. Trades some setup pain (user still needs Postgres) for Rust-native discoverability. Only worth it if Rust-curious operators show up; GHCR image covers the actual install case.
- **Architecture diagram in `docs/`** — the FTS+vector RRF hybrid and the cross-client gate state machine are both interesting enough to deserve a diagram. Skip unless someone asks. README text already covers the same ground.

## Release pipeline polish (cheap, file-and-forget)

- **Node 20 → Node 24 action bumps** — GitHub annotated the v3.2.0 build run with deprecation notices for `actions/upload-artifact@v4`, `docker/build-push-action@v6`, `docker/login-action@v3`, `docker/setup-buildx-action@v3`. Auto-forced to Node 24 on **2026-06-02**; final removal 2026-09-16. Bump to whichever versions ship native Node 24 support when the maintainers cut them. No behavior change expected, just version pins.
- **Optional release smoke-test job** — after the `merge` job, pull the just-built `:vX.Y.Z` image, boot it with an ephemeral postgres sidecar, hit `/health`, tear down. Catches "image builds but won't run" before strangers do. ~30 lines of YAML. Worth it the moment anyone reports a broken image; not worth it preemptively.
- **OCI image labels** — Dockerfile currently emits none. Adding `org.opencontainers.image.source=https://github.com/TheK3nsai/ops-brain` + `description` + `licenses=MIT OR Apache-2.0` makes the GHCR package page auto-link to the repo and surface the README. Three `LABEL` lines, no runtime cost.

## Documentation drift

- **`OPS_BRAIN_EMBEDDINGS_ENABLED` default mismatch** — `.env.example` sets it to `false`; `README.md` config table says default is `true`. The binary's actual default is what matters; align both files to that. One-commit fix, ~5 min.

## Closed (this session)

- ~~Tier 1: repo description, topics, GitHub Releases backfill, README lede + quick-start.~~ Shipped in #56.
- ~~Tier 2: GHCR multi-arch release pipeline, standalone `docker-compose.example.yml`, README quick-start refresh.~~ Shipped in #57; `v3.2.0` image live at `ghcr.io/thek3nsai/ops-brain` (public).
