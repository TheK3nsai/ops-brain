//! HTML for the bus reader. maud escapes every interpolated value; the one
//! `PreEscaped` is the markdown renderer's output, which `markdown.rs` builds
//! from escaped text only.

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use uuid::Uuid;

use super::markdown;
use crate::models::handoff::Handoff;
use crate::repo::handoff_repo::OpenActionBrief;
use crate::repo::reader_repo::{AgentLine, Cursor, HandoffLine, ThreadLine, THREAD_CAP};
use crate::tools::briefings::{STUCK_ACCEPTED_DAYS, STUCK_PENDING_DAYS};

/// Number of agent colour classes in `style.css` (`.c0` to `.c15`).
const COLOURS: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Inbox,
    Open,
    Threads,
    Timeline,
    Agents,
    None,
}

/// What every page needs: who is on the bus, who the operator is, the clock,
/// and the two counts in the navigation.
pub struct Ctx<'a> {
    pub roster: &'a [AgentLine],
    pub operator: &'a str,
    pub now: DateTime<Utc>,
    pub offset: FixedOffset,
    pub waiting: usize,
    pub open: usize,
}

impl Ctx<'_> {
    /// A slug's colour follows its position in the roster, which is ordered by
    /// first appearance, so it does not change as other slugs arrive.
    fn colour(&self, name: &str) -> usize {
        self.roster
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(name))
            .unwrap_or(0)
            % COLOURS
    }

    fn local(&self, t: DateTime<Utc>) -> DateTime<FixedOffset> {
        t.with_timezone(&self.offset)
    }

    fn stamp(&self, t: DateTime<Utc>) -> String {
        self.local(t).format("%b %-d, %H:%M").to_string()
    }

    fn zone(&self) -> String {
        if self.offset.local_minus_utc() == 0 {
            "UTC".to_string()
        } else {
            format!("UTC{}", self.offset)
        }
    }

    fn age(&self, t: DateTime<Utc>) -> String {
        age(self.now, t)
    }

    fn ago(&self, t: DateTime<Utc>) -> String {
        match self.age(t).as_str() {
            "now" => "just now".to_string(),
            other => format!("{other} ago"),
        }
    }
}

/// Compact elapsed time: `now`, `12m`, `5h`, `2d 3h`, `9d`.
pub fn age(now: DateTime<Utc>, t: DateTime<Utc>) -> String {
    let secs = (now - t).num_seconds().max(0);
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    if secs < 90 {
        "now".to_string()
    } else if mins < 60 {
        format!("{mins}m")
    } else if hours < 24 {
        format!("{hours}h")
    } else if days < 7 && hours % 24 > 0 {
        format!("{days}d {}h", hours % 24)
    } else {
        format!("{days}d")
    }
}

/// A pagination cursor as text that needs no escaping in a query string.
pub fn cursor((at, id): Cursor) -> String {
    format!("{}_{id}", at.to_rfc3339_opts(SecondsFormat::Micros, true))
}

/// The fields a list row or a chip needs, whichever query they came from.
#[derive(Clone, Copy)]
pub struct Line<'a> {
    pub id: Uuid,
    pub from: &'a str,
    pub to: Option<&'a str>,
    pub status: &'a str,
    pub accepted_by: Option<&'a str>,
    pub priority: &'a str,
    pub category: &'a str,
    pub title: &'a str,
    pub origin: &'a str,
    pub repeat_count: i32,
    pub created_at: DateTime<Utc>,
}

impl Line<'_> {
    fn is_open(&self) -> bool {
        self.category == "action" && matches!(self.status, "pending" | "accepted")
    }

    fn href(&self) -> String {
        format!("/thread/{0}#m-{0}", self.id)
    }
}

impl<'a> From<&'a Handoff> for Line<'a> {
    fn from(h: &'a Handoff) -> Self {
        Line {
            id: h.id,
            from: &h.from_agent,
            to: h.to_agent.as_deref(),
            status: &h.status,
            accepted_by: h.accepted_by.as_deref(),
            priority: &h.priority,
            category: &h.category,
            title: &h.title,
            origin: &h.origin,
            repeat_count: h.repeat_count,
            created_at: h.created_at,
        }
    }
}

impl<'a> From<&'a HandoffLine> for Line<'a> {
    fn from(h: &'a HandoffLine) -> Self {
        Line {
            id: h.id,
            from: &h.from_agent,
            to: h.to_agent.as_deref(),
            status: &h.status,
            accepted_by: h.accepted_by.as_deref(),
            priority: &h.priority,
            category: &h.category,
            title: &h.title,
            origin: &h.origin,
            repeat_count: h.repeat_count,
            created_at: h.created_at,
        }
    }
}

impl<'a> From<&'a OpenActionBrief> for Line<'a> {
    fn from(h: &'a OpenActionBrief) -> Self {
        Line {
            id: h.id,
            from: &h.from_agent,
            to: h.to_agent.as_deref(),
            status: &h.status,
            accepted_by: h.accepted_by.as_deref(),
            priority: &h.priority,
            category: "action",
            title: &h.title,
            origin: &h.origin,
            repeat_count: h.repeat_count,
            created_at: h.created_at,
        }
    }
}

// ── pieces ──

fn agent(cx: &Ctx, name: Option<&str>, small: bool) -> Markup {
    let size = if small { " sm" } else { "" };
    match name {
        None => html! { span class=(format!("ag all{size}")) { "everyone" } },
        Some(n) => {
            let op = if n.eq_ignore_ascii_case(cx.operator) {
                " op"
            } else {
                ""
            };
            html! { span class=(format!("ag c{}{size}{op}", cx.colour(n))) { i {} (n) } }
        }
    }
}

fn route(cx: &Ctx, l: &Line, small: bool) -> Markup {
    html! { (agent(cx, Some(l.from), small)) span.arrow { "→" } (agent(cx, l.to, small)) }
}

fn chips(l: &Line) -> Markup {
    let self_addressed = l.to.is_some_and(|to| to.eq_ignore_ascii_case(l.from));
    let other_acceptor = l
        .accepted_by
        .filter(|by| !l.to.is_some_and(|to| to.eq_ignore_ascii_case(by)));
    html! {
        @if l.category == "notify" {
            span.chip { "FYI" }
        } @else if l.status == "pending" {
            span.chip.pending { "pending" }
        } @else if l.status == "accepted" {
            span.chip.accepted {
                @if self_addressed { "claimed" } @else { "accepted" }
                @if let Some(by) = other_acceptor { " by " (by) }
            }
        } @else {
            span.chip.done { "completed" }
        }
        @if matches!(l.priority, "high" | "critical") {
            span.chip.hot { (l.priority) }
        } @else if l.priority != "normal" {
            span.chip { (l.priority) }
        }
        @if l.origin == "machine" { span.chip { "machine" } }
        @if l.repeat_count > 0 { span.chip { "re-filed ×" (l.repeat_count) } }
    }
}

/// One handoff as a line in a list. `clock` leads the row with a time of day
/// (timeline); without it the row ends with the handoff's age.
fn row(cx: &Ctx, l: &Line, show_to: bool, snip: Option<&str>, clock: bool) -> Markup {
    html! {
        a.row href=(l.href()) {
            @if clock { span.row-time { (cx.local(l.created_at).format("%H:%M")) } }
            span.row-main {
                span.row-title { (l.title) }
                @if let Some(s) = snip { @if !s.is_empty() { span.row-snip { (s) } } }
                span.row-meta {
                    @if show_to { (route(cx, l, true)) } @else { (agent(cx, Some(l.from), true)) }
                    (chips(l))
                }
            }
            @if !clock { span.row-age { (cx.age(l.created_at)) } }
        }
    }
}

fn list(cx: &Ctx, lines: &[Line], show_to: bool, empty: &str) -> Markup {
    html! {
        @if lines.is_empty() {
            p.empty { (empty) }
        } @else {
            div.list { @for l in lines { (row(cx, l, show_to, None, false)) } }
        }
    }
}

fn count(n: usize) -> Markup {
    html! { span.count { (n) } }
}

pub fn layout(cx: &Ctx, title: &str, nav: Nav, query: &str, body: Markup) -> Markup {
    let tab = |target: Nav, href: &str, label: &str, badge: Option<(usize, bool)>| {
        html! {
            a.on[nav == target] href=(href) {
                (label)
                @if let Some((n, due)) = badge { @if n > 0 { b.due[due] { (n) } } }
            }
        }
    };
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width,initial-scale=1";
                title { (title) " · ops-brain" }
                link rel="stylesheet" href="/style.css";
            }
            body {
                header.top {
                    a.brand href="/" { "ops-brain" span { "bus reader" } }
                    nav {
                        (tab(Nav::Inbox, "/", "Inbox", Some((cx.waiting, true))))
                        (tab(Nav::Open, "/open", "Open", Some((cx.open, false))))
                        (tab(Nav::Threads, "/threads", "Threads", None))
                        (tab(Nav::Timeline, "/timeline", "Timeline", None))
                        (tab(Nav::Agents, "/agents", "Agents", None))
                    }
                    form.search action="/search" method="get" role="search" {
                        input type="search" name="q" value=(query) placeholder="Search"
                            aria-label="Search handoffs" maxlength="200";
                    }
                }
                main { (body) }
                p.foot {
                    "Read-only. Rendered " (cx.stamp(cx.now)) ", times in " (cx.zone()) "."
                }
            }
        }
    }
}

// ── pages ──

/// One item waiting on the operator, with the opening of its body when the
/// reader could fetch it.
pub struct Waiting<'a> {
    pub line: Line<'a>,
    pub snippet: Option<String>,
}

pub fn inbox(
    cx: &Ctx,
    waiting: &[Waiting],
    failing: &[Line],
    stuck: &[Line],
    operator_seen: bool,
) -> Markup {
    let body = html! {
        h1 { "Waiting on you" (count(waiting.len())) }
        p.lede {
            "Open action handoffs addressed to " b { (cx.operator) } ", oldest first."
        }
        @if waiting.is_empty() && !operator_seen {
            p.warn {
                "No handoff has ever named " b { (cx.operator) } ". If your agents address "
                "you by another slug, set OPS_BRAIN_READER_OPERATOR to it; until then this "
                "section cannot show what is waiting."
            }
        } @else if waiting.is_empty() {
            p.empty { "Nothing is waiting on you." }
        } @else {
            div.cards {
                @for w in waiting {
                    a.card href=(w.line.href()) {
                        span.card-top {
                            (agent(cx, Some(w.line.from), false))
                            span.card-age { "open " (cx.age(w.line.created_at)) }
                        }
                        span.card-title { (w.line.title) }
                        @if let Some(s) = &w.snippet { span.card-snip { (s) } }
                        span.card-meta {
                            (chips(&w.line))
                            span.muted { "filed " (cx.stamp(w.line.created_at)) }
                        }
                    }
                }
            }
        }
        h2 { "Failing checks" (count(failing.len())) }
        (list(cx, failing, true, "No open machine finding rated high or critical."))
        h2 { "Stuck" (count(stuck.len())) }
        (list(cx, stuck, true, &format!(
            "Nothing pending over {STUCK_PENDING_DAYS} days or accepted over {STUCK_ACCEPTED_DAYS}."
        )))
    };
    layout(cx, "Inbox", Nav::Inbox, "", body)
}

pub fn open(cx: &Ctx, groups: &[(String, Vec<Line>)], locks: &[Line]) -> Markup {
    let body = html! {
        h1 { "Open work" (count(cx.open)) }
        p.lede { "Every action handoff not yet completed, grouped by who owes it." }
        @if groups.is_empty() { p.empty { "The bus is clear." } }
        @for (name, lines) in groups {
            div.group {
                h3 { (agent(cx, Some(name), false)) (count(lines.len())) }
                (list(cx, lines, false, ""))
            }
        }
        @if !locks.is_empty() {
            h2 { "Locks held" (count(locks.len())) }
            p.lede {
                "Self-addressed claims: an agent marking work as its own so a peer does "
                "not collide with it."
            }
            (list(cx, locks, false, ""))
        }
    };
    layout(cx, "Open work", Nav::Open, "", body)
}

pub fn threads(cx: &Ctx, items: &[ThreadLine], older: Option<String>) -> Markup {
    let body = html! {
        h1 { "Threads" }
        p.lede {
            "Conversations between agents, most recent activity first. The number is "
            "messages in the thread; a handoff nobody replied to is on the timeline."
        }
        @if items.is_empty() {
            p.empty { "No handoff has a reply yet." }
        } @else {
            div.list {
                @for t in items {
                    a.row href=(format!("/thread/{}", t.root)) {
                        span.row-main {
                            span.row-title { (t.title) }
                            span.row-meta {
                                @for p in participants(t) { (agent(cx, Some(p), true)) }
                                @if t.open > 0 { span.chip.pending { (t.open) " open" } }
                            }
                        }
                        span.row-n { (t.messages) }
                        span.row-age { (cx.age(t.last_at)) }
                    }
                }
            }
        }
        @if let Some(before) = older {
            a.pager href=(format!("/threads?before={before}")) { "Older threads →" }
        }
    };
    layout(cx, "Threads", Nav::Threads, "", body)
}

/// Root sender, root recipient, then everyone else who wrote, without repeats.
fn participants(t: &ThreadLine) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let names = [Some(t.from_agent.as_str()), t.to_agent.as_deref()]
        .into_iter()
        .flatten()
        .chain(t.senders.iter().map(String::as_str));
    for name in names {
        if !out.iter().any(|seen| seen.eq_ignore_ascii_case(name)) {
            out.push(name);
        }
    }
    out.truncate(6);
    out
}

/// `cut` says the thread had more messages than `messages` holds.
pub fn thread(cx: &Ctx, messages: &[Handoff], cut: bool) -> Markup {
    let root = &messages[0];
    let open = messages.iter().filter(|m| Line::from(*m).is_open()).count();
    let last = &messages[messages.len() - 1];
    let body = html! {
        a.back href="/threads" { "← Threads" }
        h1 { (root.title) }
        p.lede {
            (messages.len()) @if messages.len() == 1 { " message" } @else { " messages" }
            " · " (cx.stamp(root.created_at))
            @if messages.len() > 1 { " – " (cx.stamp(last.created_at)) }
            " · "
            @if open > 0 { span.chip.pending { (open) " open" } } @else { "closed" }
        }
        @if cut {
            p.warn { "This thread is longer than one page shows; the first " (THREAD_CAP) " messages follow." }
        }
        @for (i, m) in messages.iter().enumerate() {
            (message(cx, m, i.checked_sub(1).map(|p| &messages[p]), messages))
        }
    };
    layout(cx, &root.title, Nav::Threads, "", body)
}

fn message(cx: &Ctx, m: &Handoff, previous: Option<&Handoff>, all: &[Handoff]) -> Markup {
    let line = Line::from(m);
    // Say what a message answers only when it is not simply the one above it.
    let parent = m
        .in_reply_to
        .filter(|p| previous.map(|prev| prev.id) != Some(*p))
        .and_then(|p| all.iter().find(|x| x.id == p));
    let context = m
        .context
        .as_ref()
        .and_then(|c| serde_json::to_string_pretty(c).ok());
    html! {
        article.msg.is-open[line.is_open()] id=(format!("m-{}", m.id)) {
            header {
                (route(cx, &line, false))
                time datetime=(m.created_at.to_rfc3339()) {
                    (cx.stamp(m.created_at)) " · " (cx.ago(m.created_at))
                }
            }
            h3 { (m.title) " " (chips(&line)) }
            @if let Some(p) = parent {
                a.replyto href=(format!("#m-{}", p.id)) {
                    "↳ in reply to " (p.from_agent) ", " (cx.stamp(p.created_at))
                }
            }
            div.body { (PreEscaped(markdown::render(&m.body))) }
            @if let Some(json) = context {
                details.ctx { summary { "context" } pre { (json) } }
            }
            footer {
                span.uuid { (m.id) }
                @if let Some(commit) = &m.commit_hash { span.commit { "commit " (commit) } }
                @if !line.is_open() && m.category == "action" {
                    span.muted { "closed " (cx.stamp(m.updated_at)) }
                }
            }
        }
    }
}

pub fn timeline(cx: &Ctx, lines: &[HandoffLine], older: Option<String>) -> Markup {
    let day = |l: &HandoffLine| cx.local(l.created_at).format("%A, %b %-d").to_string();
    let body = html! {
        h1 { "Timeline" }
        p.lede { "Everything the bus carried, newest first." }
        @if lines.is_empty() { p.empty { "Nothing here." } }
        @for group in lines.chunk_by(|a, b| day(a) == day(b)) {
            h2.day { (day(&group[0])) (count(group.len())) }
            div.list {
                @for l in group { (row(cx, &Line::from(l), true, None, true)) }
            }
        }
        @if let Some(before) = older {
            a.pager href=(format!("/timeline?before={before}")) { "Older handoffs →" }
        }
    };
    layout(cx, "Timeline", Nav::Timeline, "", body)
}

pub fn agents(cx: &Ctx) -> Markup {
    let mut by_traffic: Vec<&AgentLine> = cx.roster.iter().collect();
    by_traffic.sort_by_key(|a| std::cmp::Reverse(a.filed + a.received));
    let body = html! {
        h1 { "Agents" (count(cx.roster.len())) }
        p.lede {
            "Every slug a handoff has named. This is read off the handoffs themselves: "
            "there is no registry, no heartbeat and no presence."
        }
        div.agents {
            @for a in by_traffic {
                div.acard {
                    div.acard-top {
                        (agent(cx, Some(&a.name), false))
                        span.muted {
                            @if a.name.eq_ignore_ascii_case(cx.operator) { "operator" }
                            @else if a.machine_only == Some(true) { "machine producer" }
                            @else { "agent" }
                        }
                    }
                    dl {
                        div { dt { "filed" } dd { (a.filed) } }
                        div { dt { "received" } dd { (a.received) } }
                        div { dt { "owes" } dd.due[a.owes > 0] { (a.owes) } }
                    }
                    p.muted {
                        @match a.last_filed {
                            Some(t) => { "last filed " (cx.ago(t)) }
                            None => { "has not filed" }
                        }
                    }
                }
            }
        }
    };
    layout(cx, "Agents", Nav::Agents, "", body)
}

pub fn search(cx: &Ctx, query: &str, results: &[Handoff]) -> Markup {
    let body = html! {
        h1 { "Search" @if !query.is_empty() { (count(results.len())) } }
        @if query.is_empty() {
            p.lede {
                "Full-text search over handoff titles and bodies. A \"quoted phrase\", "
                code { "or" } " and " code { "-word" } " work as in a web search."
            }
        } @else if results.is_empty() {
            p.empty { "Nothing matches “" (query) "”." }
        } @else {
            p.lede { "Best match first for “" (query) "”." }
            div.list {
                @for h in results {
                    (row(cx, &Line::from(h), true, Some(&markdown::snippet(&h.body, 160)), false))
                }
            }
        }
    };
    layout(cx, "Search", Nav::None, query, body)
}

pub fn notice(cx: &Ctx, title: &str, text: &str) -> Markup {
    let body = html! {
        h1 { (title) }
        p.lede { (text) }
        a.pager href="/" { "← Inbox" }
    };
    layout(cx, title, Nav::None, "", body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn age_reads_compactly_at_every_scale() {
        let now = at("2026-11-15T12:00:00Z");
        for (t, want) in [
            ("2026-11-15T11:59:30Z", "now"),
            ("2026-11-15T11:48:00Z", "12m"),
            ("2026-11-15T07:00:00Z", "5h"),
            ("2026-11-13T09:00:00Z", "2d 3h"),
            ("2026-11-13T12:00:00Z", "2d"),
            ("2026-11-06T10:00:00Z", "9d"),
            // A producer's clock running ahead reads as now, not negative.
            ("2026-11-15T12:05:00Z", "now"),
        ] {
            assert_eq!(age(now, at(t)), want, "{t}");
        }
    }

    #[test]
    fn cursor_needs_no_query_escaping() {
        let c = cursor((at("2026-11-13T16:50:01.123456Z"), Uuid::nil()));
        assert_eq!(
            c,
            "2026-11-13T16:50:01.123456Z_00000000-0000-0000-0000-000000000000"
        );
        assert!(c
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)));
    }
}
