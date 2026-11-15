//! The bus reader: a read-only web view of the handoffs for the person who
//! runs the deployment. Contract and operator setup: `docs/reader.md`.
//!
//! It is a second HTTP listener, off unless `OPS_BRAIN_READER_LISTEN` is set,
//! and shares nothing with `/mcp` or `/api` but the database pool. Three
//! properties are structural rather than promised:
//!
//! - **Read-only.** Every route is a GET and no handler writes. The reader has
//!   no agent identity, so it could not file a handoff if it tried.
//! - **Nothing to execute.** Pages carry no script, and the response policy
//!   allows none, so a handoff body cannot run code in the operator's browser.
//! - **No agent cost.** It adds no MCP tool and no word to the instructions.

pub mod markdown;
mod views;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use base64::Engine;
use chrono::{DateTime, FixedOffset, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::{validate_token, MIN_TOKEN_LEN};
use crate::repo::handoff_repo::{self, OpenActionBrief};
use crate::repo::reader_repo::{self, AgentLine, Cursor, THREAD_CAP};
use crate::tools::briefings::{self, Section};
use crate::validation::validate_agent_name;
use views::{Ctx, Line, Waiting};

const THREADS_PAGE: usize = 100;
const TIMELINE_PAGE: usize = 200;
const SEARCH_LIMIT: i64 = 50;
const MAX_QUERY_CHARS: usize = 200;

/// Validated reader settings. `Debug` redacts the token.
#[derive(Clone)]
pub struct ReaderConfig {
    pub listen: String,
    /// Password for HTTP Basic authentication. `None` only on a loopback bind.
    pub token: Option<String>,
    /// Slug whose open queue is the inbox.
    pub operator: String,
    /// Offset the pages render timestamps in.
    pub offset: FixedOffset,
}

impl std::fmt::Debug for ReaderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderConfig")
            .field("listen", &self.listen)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("operator", &self.operator)
            .field("offset", &self.offset)
            .finish()
    }
}

fn binds_loopback(listen: &str) -> bool {
    listen
        .parse::<SocketAddr>()
        .is_ok_and(|addr| addr.ip().is_loopback())
}

/// Validate the reader's settings. `Ok(None)` means it is switched off.
///
/// Fails closed: a listener other processes or containers can reach must have
/// a token. A loopback address in a container is still loopback, so the rule
/// holds there too; `0.0.0.0` behind a published port is the case it catches.
/// `other_secrets` are the bearer credentials already configured, which the
/// reader token must not repeat: a browser password that is also an agent
/// token would blur the credential classes.
pub fn configure(
    listen: Option<&str>,
    token: Option<&str>,
    operator: Option<&str>,
    utc_offset: Option<&str>,
    other_secrets: &[&str],
) -> Result<Option<ReaderConfig>, String> {
    fn clean(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    let Some(listen) = clean(listen) else {
        return Ok(None);
    };
    let token = clean(token);
    match token {
        Some(t) if t.len() < MIN_TOKEN_LEN => {
            return Err(format!(
                "OPS_BRAIN_READER_TOKEN is too short ({} chars, min {MIN_TOKEN_LEN})",
                t.len()
            ));
        }
        Some(t) if other_secrets.contains(&t) => {
            return Err(
                "OPS_BRAIN_READER_TOKEN repeats another configured token; mint a separate one"
                    .to_string(),
            );
        }
        None if !binds_loopback(listen) => {
            return Err(format!(
                "OPS_BRAIN_READER_LISTEN={listen} is not a loopback address, so \
                 OPS_BRAIN_READER_TOKEN is required; without it anything that can reach the \
                 port could read every handoff"
            ));
        }
        _ => {}
    }
    let operator = match clean(operator) {
        Some(raw) => {
            validate_agent_name(raw).map_err(|e| format!("OPS_BRAIN_READER_OPERATOR: {e}"))?
        }
        None => briefings::DEFAULT_OPERATOR,
    };
    let offset = match clean(utc_offset) {
        Some(raw) => raw.parse::<FixedOffset>().map_err(|e| {
            format!("OPS_BRAIN_READER_UTC_OFFSET '{raw}': {e} (expected a form like -04:00)")
        })?,
        None => FixedOffset::east_opt(0).expect("zero is a valid offset"),
    };
    Ok(Some(ReaderConfig {
        listen: listen.to_string(),
        token: token.map(str::to_string),
        operator: operator.to_string(),
        offset,
    }))
}

#[derive(Clone)]
pub struct ReaderState {
    pool: PgPool,
    cfg: Arc<ReaderConfig>,
}

pub fn router(pool: PgPool, cfg: ReaderConfig) -> Router {
    let state = ReaderState {
        pool,
        cfg: Arc::new(cfg),
    };
    Router::new()
        .route("/", get(inbox))
        .route("/open", get(open))
        .route("/threads", get(threads))
        .route("/thread/:id", get(thread))
        .route("/timeline", get(timeline))
        .route("/agents", get(agents))
        .route("/search", get(search))
        .route("/style.css", get(style))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

// ── access and response policy ──

/// The password of an HTTP Basic credential. The username is ignored.
fn basic_password(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, encoded) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let credential = String::from_utf8(bytes).ok()?;
    credential.split_once(':').map(|(_, pass)| pass.to_string())
}

/// Without a token the reader answers only when the browser addressed it by a
/// loopback name. That is what stops a page on another site from reading the
/// bus through DNS rebinding: such a request carries the attacker's hostname.
fn loopback_host(headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

/// The refusal for a request that may not read the bus, or `None` to serve it.
fn rejection(cfg: &ReaderConfig, headers: &HeaderMap) -> Option<Response> {
    let Some(expected) = &cfg.token else {
        return (!loopback_host(headers)).then(|| {
            (
                StatusCode::FORBIDDEN,
                "The reader is running without a token, so it answers only on localhost. \
                 Set OPS_BRAIN_READER_TOKEN to reach it by another name.\n",
            )
                .into_response()
        });
    };
    let presented = basic_password(headers);
    if presented
        .as_deref()
        .is_some_and(|p| validate_token(p, expected))
    {
        return None;
    }
    if presented.is_some() {
        tracing::warn!("reader: a wrong token was presented");
    }
    Some(
        (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                r#"Basic realm="ops-brain reader", charset="UTF-8""#,
            )],
            "Sign in with any username and the reader token as the password.\n",
        )
            .into_response(),
    )
}

/// `default-src 'none'` is the load-bearing line: no script, image, frame or
/// connection is allowed, whatever a handoff body manages to say.
const RESPONSE_POLICY: [(&str, &str); 5] = [
    (
        "content-security-policy",
        "default-src 'none'; style-src 'self'; form-action 'self'; base-uri 'none'; \
         frame-ancestors 'none'",
    ),
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "no-referrer"),
    ("x-robots-tag", "noindex, nofollow"),
];

async fn guard(State(state): State<ReaderState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = match rejection(&state.cfg, request.headers()) {
        Some(refused) => refused,
        None => next.run(request).await,
    };
    let headers = response.headers_mut();
    for (name, value) in RESPONSE_POLICY {
        headers.insert(name, HeaderValue::from_static(value));
    }
    // Handoffs may carry client detail; keep them out of the browser's disk
    // cache. The stylesheet sets its own lifetime.
    headers
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    // One line per page opened: the record of whether the reader is used.
    if response.status().is_success() && path != "/style.css" {
        tracing::info!(%path, "reader view");
    }
    response
}

// ── handlers ──

/// What every page loads first: the roster for colours and the open set for
/// the two counts in the navigation.
struct Chrome {
    roster: Vec<AgentLine>,
    briefs: Vec<OpenActionBrief>,
    now: DateTime<Utc>,
}

/// An accepted self-addressed handoff is a lock, not work (`NOT_SELF_CLAIM`).
fn is_lock(row: &OpenActionBrief) -> bool {
    row.status == "accepted" && row.is_self_addressed
}

impl Chrome {
    async fn load(state: &ReaderState) -> Result<Self, sqlx::Error> {
        let (roster, briefs) = tokio::try_join!(
            reader_repo::agent_roster(&state.pool),
            handoff_repo::list_open_action_briefs(&state.pool, &state.cfg.operator),
        )?;
        Ok(Chrome {
            roster,
            briefs,
            now: Utc::now(),
        })
    }

    fn ctx<'a>(&'a self, state: &'a ReaderState) -> Ctx<'a> {
        Ctx {
            roster: &self.roster,
            operator: &state.cfg.operator,
            now: self.now,
            offset: state.cfg.offset,
            waiting: self.briefs.iter().filter(|r| r.is_operator).count(),
            open: self.briefs.iter().filter(|r| !is_lock(r)).count(),
        }
    }
}

type Page = Result<Response, sqlx::Error>;

fn page(markup: maud::Markup) -> Response {
    Html(markup.into_string()).into_response()
}

fn respond(result: Page) -> Response {
    result.unwrap_or_else(|error| {
        tracing::error!(%error, "reader: query failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "The reader could not read the database. The server log has the error.\n",
        )
            .into_response()
    })
}

/// A page that is worth leaving open: ask the browser to reload it.
fn refreshing(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert("refresh", HeaderValue::from_static("300"));
    response
}

async fn inbox(State(state): State<ReaderState>) -> Response {
    refreshing(respond(inbox_page(&state).await))
}

async fn inbox_page(state: &ReaderState) -> Page {
    let chrome = Chrome::load(state).await?;
    let cx = chrome.ctx(state);
    let section = |want: Section| {
        chrome
            .briefs
            .iter()
            .filter(move |r| briefings::section_of(r, &chrome.now) == want)
    };
    // The open set carries no bodies; fetch the waiting ones for snippets.
    let mine: Vec<&OpenActionBrief> = section(Section::WaitingOnYou).collect();
    let ids: Vec<Uuid> = mine.iter().map(|r| r.id).collect();
    let bodies = reader_repo::bodies(&state.pool, &ids).await?;
    let waiting: Vec<Waiting> = mine
        .into_iter()
        .map(|r| Waiting {
            line: Line::from(r),
            snippet: bodies
                .iter()
                .find(|(id, _)| *id == r.id)
                .map(|(_, body)| markdown::snippet(body, 240)),
        })
        .collect();
    let failing: Vec<Line> = section(Section::FailingCheck).map(Line::from).collect();
    let stuck: Vec<Line> = section(Section::Stuck).map(Line::from).collect();
    // An empty queue for a slug no handoff has ever named is a misconfigured
    // reader, not good news (the briefing draws the same distinction).
    let operator_seen = chrome
        .roster
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(&state.cfg.operator));
    Ok(page(views::inbox(
        &cx,
        &waiting,
        &failing,
        &stuck,
        operator_seen,
    )))
}

async fn open(State(state): State<ReaderState>) -> Response {
    refreshing(respond(open_page(&state).await))
}

async fn open_page(state: &ReaderState) -> Page {
    let chrome = Chrome::load(state).await?;
    let cx = chrome.ctx(state);
    let mut groups: Vec<(String, Vec<Line>)> = Vec::new();
    let mut locks: Vec<Line> = Vec::new();
    for row in &chrome.briefs {
        if is_lock(row) {
            locks.push(Line::from(row));
            continue;
        }
        let owner = briefings::recipient(row);
        match groups
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case(owner))
        {
            Some((_, lines)) => lines.push(Line::from(row)),
            None => groups.push((owner.to_string(), vec![Line::from(row)])),
        }
    }
    // Busiest queue first; the name keeps the order stable between reloads.
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    Ok(page(views::open(&cx, &groups, &locks)))
}

#[derive(Deserialize)]
struct Before {
    before: Option<String>,
}

impl Before {
    /// A cursor that does not parse reads as the first page.
    fn parse(&self) -> Option<Cursor> {
        let (at, id) = self.before.as_deref()?.split_once('_')?;
        Some((at.parse().ok()?, id.parse().ok()?))
    }
}

/// Split a `limit + 1` fetch into the page and the cursor for the next one.
fn paginate<T>(
    mut rows: Vec<T>,
    limit: usize,
    key: impl Fn(&T) -> Cursor,
) -> (Vec<T>, Option<String>) {
    if rows.len() <= limit {
        return (rows, None);
    }
    rows.truncate(limit);
    let older = rows.last().map(|last| views::cursor(key(last)));
    (rows, older)
}

async fn threads(State(state): State<ReaderState>, Query(q): Query<Before>) -> Response {
    respond(threads_page(&state, q.parse()).await)
}

async fn threads_page(state: &ReaderState, before: Option<Cursor>) -> Page {
    let chrome = Chrome::load(state).await?;
    let rows = reader_repo::list_threads(&state.pool, before, THREADS_PAGE as i64 + 1).await?;
    let (rows, older) = paginate(rows, THREADS_PAGE, |t| (t.last_at, t.root));
    Ok(page(views::threads(&chrome.ctx(state), &rows, older)))
}

async fn thread(State(state): State<ReaderState>, Path(id): Path<String>) -> Response {
    respond(thread_page(&state, &id).await)
}

async fn thread_page(state: &ReaderState, id: &str) -> Page {
    let chrome = Chrome::load(state).await?;
    let cx = chrome.ctx(state);
    let mut messages = match Uuid::parse_str(id) {
        Ok(id) => reader_repo::get_thread(&state.pool, id).await?,
        Err(_) => Vec::new(),
    };
    let cut = messages.len() > THREAD_CAP;
    messages.truncate(THREAD_CAP);
    if messages.is_empty() {
        let body = views::notice(
            &cx,
            "No such handoff",
            "Nothing on the bus has that id. It may have been deleted.",
        );
        return Ok((StatusCode::NOT_FOUND, Html(body.into_string())).into_response());
    }
    Ok(page(views::thread(&cx, &messages, cut)))
}

async fn timeline(State(state): State<ReaderState>, Query(q): Query<Before>) -> Response {
    respond(timeline_page(&state, q.parse()).await)
}

async fn timeline_page(state: &ReaderState, before: Option<Cursor>) -> Page {
    let chrome = Chrome::load(state).await?;
    let rows = reader_repo::list_lines(&state.pool, before, TIMELINE_PAGE as i64 + 1).await?;
    let (rows, older) = paginate(rows, TIMELINE_PAGE, |l| (l.created_at, l.id));
    Ok(page(views::timeline(&chrome.ctx(state), &rows, older)))
}

async fn agents(State(state): State<ReaderState>) -> Response {
    respond(
        async {
            Ok(page(views::agents(
                &Chrome::load(&state).await?.ctx(&state),
            )))
        }
        .await,
    )
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

async fn search(State(state): State<ReaderState>, Query(q): Query<SearchQuery>) -> Response {
    let query: String =
        q.q.as_deref()
            .unwrap_or_default()
            .trim()
            .chars()
            .take(MAX_QUERY_CHARS)
            .collect();
    respond(search_page(&state, &query).await)
}

async fn search_page(state: &ReaderState, query: &str) -> Page {
    let chrome = Chrome::load(state).await?;
    let results = if query.is_empty() {
        Vec::new()
    } else {
        handoff_repo::search_handoffs(&state.pool, query, None, SEARCH_LIMIT).await?
    };
    Ok(page(views::search(&chrome.ctx(state), query, &results)))
}

async fn style() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "max-age=300"),
        ],
        include_str!("style.css"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn basic(user: &str, pass: &str) -> String {
        let raw = format!("{user}:{pass}");
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        )
    }

    #[test]
    fn unset_or_blank_listen_switches_the_reader_off() {
        assert!(configure(None, Some(TOKEN), None, None, &[])
            .unwrap()
            .is_none());
        assert!(configure(Some("  "), None, None, None, &[])
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_reachable_listener_without_a_token_refuses_to_start() {
        for listen in ["0.0.0.0:3210", "[::]:3210", "10.0.0.5:3210", "reader:3210"] {
            let err = configure(Some(listen), None, None, None, &[]).unwrap_err();
            assert!(err.contains("OPS_BRAIN_READER_TOKEN is required"), "{err}");
            // A blank token is no token.
            assert!(configure(Some(listen), Some(" "), None, None, &[]).is_err());
            assert!(configure(Some(listen), Some(TOKEN), None, None, &[]).is_ok());
        }
    }

    #[test]
    fn loopback_may_run_without_a_token() {
        for listen in ["127.0.0.1:3210", "[::1]:3210"] {
            let cfg = configure(Some(listen), None, None, None, &[])
                .unwrap()
                .unwrap();
            assert!(cfg.token.is_none());
            assert_eq!(cfg.operator, briefings::DEFAULT_OPERATOR);
            assert_eq!(cfg.offset.local_minus_utc(), 0);
        }
    }

    #[test]
    fn token_must_be_long_and_its_own() {
        let short = configure(Some("127.0.0.1:1"), Some("hunter2"), None, None, &[]);
        assert!(short.unwrap_err().contains("too short"));
        let reused = configure(Some("127.0.0.1:1"), Some(TOKEN), None, None, &[TOKEN]);
        assert!(reused.unwrap_err().contains("repeats another"));
    }

    #[test]
    fn operator_and_offset_are_validated() {
        let cfg = configure(
            Some("127.0.0.1:1"),
            None,
            Some(" Pat "),
            Some("-04:00"),
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(cfg.operator, "Pat");
        assert_eq!(cfg.offset.local_minus_utc(), -4 * 3600);
        assert!(configure(Some("127.0.0.1:1"), None, Some("no spaces"), None, &[]).is_err());
        assert!(configure(Some("127.0.0.1:1"), None, None, Some("AST"), &[]).is_err());
    }

    #[test]
    fn debug_never_prints_the_token() {
        let cfg = configure(Some("0.0.0.0:1"), Some(TOKEN), None, None, &[])
            .unwrap()
            .unwrap();
        assert!(!format!("{cfg:?}").contains(TOKEN));
    }

    #[test]
    fn basic_password_ignores_the_username_and_keeps_colons() {
        let h = headers(&[("authorization", &basic("anyone", "se:cret"))]);
        assert_eq!(basic_password(&h).as_deref(), Some("se:cret"));
        let lower = basic("", "x").replacen("Basic", "basic", 1);
        assert_eq!(
            basic_password(&headers(&[("authorization", &lower)])).as_deref(),
            Some("x")
        );
        for bad in ["Bearer abc", "Basic !!!", "Basic", "Basic bm9jb2xvbg=="] {
            assert!(
                basic_password(&headers(&[("authorization", bad)])).is_none(),
                "{bad}"
            );
        }
        assert!(basic_password(&HeaderMap::new()).is_none());
    }

    #[test]
    fn loopback_host_rejects_any_other_name() {
        for ok in [
            "localhost",
            "localhost:3210",
            "LOCALHOST:3210",
            "127.0.0.1:3210",
            "[::1]:3210",
        ] {
            assert!(loopback_host(&headers(&[("host", ok)])), "{ok}");
        }
        for bad in [
            "evil.example",
            "evil.example:3210",
            "localhost.evil.example",
            "127.0.0.1.evil.example:3210",
            "[::2]:3210",
            "",
        ] {
            assert!(!loopback_host(&headers(&[("host", bad)])), "{bad}");
        }
        assert!(!loopback_host(&HeaderMap::new()));
    }

    #[test]
    fn paginate_reports_a_cursor_only_when_more_rows_exist() {
        let key = |n: &i64| (DateTime::from_timestamp(*n, 0).unwrap(), Uuid::nil());
        let (rows, older) = paginate(vec![3, 2, 1], 3, key);
        assert_eq!((rows.len(), older), (3, None));
        let (rows, older) = paginate(vec![4, 3, 2, 1], 3, key);
        assert_eq!(rows, vec![4, 3, 2]);
        let older = Before { before: older };
        assert_eq!(older.parse(), Some(key(&2)));
        for junk in [
            "",
            "yesterday",
            "1970-01-01T00:00:02Z",
            "1970-01-01T00:00:02Z_x",
        ] {
            let before = Some(junk.to_string());
            assert_eq!(Before { before }.parse(), None, "{junk}");
        }
    }
}
