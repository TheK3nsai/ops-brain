//! Handoff bodies are markdown written by agents, some of which read mail and
//! web pages. Rendering treats them as hostile: raw HTML becomes text, images
//! are never emitted (an image URL is a read receipt), and a link survives
//! only with an http, https or mailto destination.

use pulldown_cmark::{html, Alignment, Event, LinkType, Options, Parser, Tag, TagEnd};

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH
}

fn safe_url(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/// Render a handoff body to HTML that carries no markup of its author's.
pub fn render(src: &str) -> String {
    // One entry per open link: was its opening tag dropped?
    let mut dropped: Vec<bool> = Vec::new();
    let events = Parser::new_ext(src, options()).filter_map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Some(Event::Text(raw)),
        Event::Start(Tag::Link {
            link_type,
            ref dest_url,
            ..
        }) => {
            // `<ops@example.com>` arrives as a bare address; the writer adds
            // the `mailto:` itself.
            let drop = link_type != LinkType::Email && !safe_url(dest_url);
            dropped.push(drop);
            (!drop).then_some(event)
        }
        Event::End(TagEnd::Link) => (!dropped.pop().unwrap_or(true)).then_some(event),
        // Column alignment is written as an inline style, which the response
        // policy refuses; drop it rather than have the browser report it.
        Event::Start(Tag::Table(columns)) => Some(Event::Start(Tag::Table(vec![
            Alignment::None;
            columns.len()
        ]))),
        // The alt text between these two stays, as plain text.
        Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => None,
        other => Some(other),
    });
    let mut out = String::with_capacity(src.len() + src.len() / 2);
    html::push_html(&mut out, events);
    out
}

/// The first `max` characters of a body as plain text, for a list row.
pub fn snippet(src: &str, max: usize) -> String {
    let mut out = String::new();
    for event in Parser::new_ext(src, options()) {
        match event {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak | Event::End(_)
                if !out.ends_with(' ') && !out.is_empty() =>
            {
                out.push(' ');
            }
            _ => {}
        }
        if out.chars().count() > max {
            break;
        }
    }
    let trimmed = out.trim_end();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut cut: String = trimmed.chars().take(max).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_html_is_escaped_not_emitted() {
        let html = render("hello <script>alert(1)</script>\n\n<div onclick=\"x()\">block</div>");
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<div"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(html.contains("&lt;div"), "{html}");
    }

    #[test]
    fn script_and_data_links_lose_their_anchor_but_keep_their_text() {
        for dest in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            " javascript:alert(1)",
            "data:text/html,x",
            "vbscript:x",
            "/open",
            "#m-1",
        ] {
            let html = render(&format!("[click me]({dest})"));
            assert!(!html.contains("<a"), "{dest} kept an anchor: {html}");
            assert!(html.contains("click me"), "{html}");
        }
    }

    #[test]
    fn web_and_mail_links_survive() {
        let html = render(
            "[pr](https://example.com/pull/1), <mailto:ops@example.com> and <dev@example.com>",
        );
        assert!(html.contains(r#"href="mailto:dev@example.com""#), "{html}");
        assert!(
            html.contains(r#"<a href="https://example.com/pull/1">pr</a>"#),
            "{html}"
        );
        assert!(html.contains(r#"href="mailto:ops@example.com""#), "{html}");
    }

    #[test]
    fn images_are_never_emitted() {
        let html = render("![proof](https://tracker.example/pixel.png) [![a](https://x.example/i)](https://example.com)");
        assert!(!html.contains("<img"), "{html}");
        assert!(!html.contains("pixel.png"), "{html}");
        assert!(html.contains("proof"), "{html}");
        assert!(
            html.contains(r#"<a href="https://example.com">a</a>"#),
            "{html}"
        );
    }

    #[test]
    fn tables_and_code_render() {
        let html = render("| a | b |\n|:--|--:|\n| 1 | 2 |\n\n```\nlet x = \"<b>\";\n```");
        assert!(html.contains("<table>"), "{html}");
        assert!(!html.contains("style="), "{html}");
        assert!(html.contains("&lt;b&gt;"), "{html}");
    }

    #[test]
    fn snippet_is_plain_text_and_bounded() {
        let s = snippet(
            "## Done so far\n\nI accepted `01a1d5cc`, **which** goes with [this](https://x.example).",
            40,
        );
        assert!(
            !s.contains('#') && !s.contains('*') && !s.contains('`'),
            "{s}"
        );
        assert!(s.starts_with("Done so far I accepted 01a1d5cc"), "{s}");
        assert!(s.ends_with('…') && s.chars().count() <= 41, "{s}");
        assert_eq!(snippet("short", 40), "short");
        assert_eq!(snippet("", 40), "");
    }
}
