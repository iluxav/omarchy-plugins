//! One pipeline for every body: HTML → markdown, drawn by omanote's renderer.
//!
//! Emails use tables for layout, so tables are flattened to paragraphs rather
//! than turned into markdown tables. Images are dropped: no remote image in a
//! message is ever loaded, and tracking pixels go with them.

use htmd::HtmlToMarkdown;
use htmd::element_handler::{HandlerResult, Handlers};

use crate::markdown::{Block, classify, style_line};

fn block(content: &str) -> HandlerResult {
    let content = content.trim_matches(|c: char| c == '\n' || c.is_whitespace());
    if content.is_empty() { "".into() } else { format!("\n\n{content}\n\n").into() }
}

// htmd with custom handlers is not Sync, so each thread builds its own.
thread_local! {
    static LOCAL: HtmlToMarkdown = build();
}

fn build() -> HtmlToMarkdown {
    HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "head", "title", "meta", "link", "img", "picture", "svg", "video", "audio", "iframe", "object", "form", "button"])
        .add_handler(vec!["table", "tbody", "thead", "tfoot", "tr", "td", "th", "center"], |h: &dyn Handlers, el: htmd::Element| {
            Some(block(&h.walk_children(el.node).content))
        })
        .build()
}

pub fn html_to_md(html: &str) -> String {
    let md = LOCAL.with(|c| c.convert(html)).unwrap_or_default();
    tidy(&md)
}

/// Invisible padding newsletters stuff their preheaders with, and runs of blank lines.
pub fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let line: String = line
            .chars()
            .filter(|c| !matches!(c, '\u{034f}' | '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}' | '\u{00ad}' | '\r'))
            .map(|c| if c == '\u{a0}' { ' ' } else { c })
            .collect();
        let line = line.trim_end();
        // A line of nothing but padding or escaped spaces is blank.
        if line.trim().is_empty() || line.trim().chars().all(|c| c == '\\' || c == ' ') {
            blank += 1;
            continue;
        }
        if !out.is_empty() && blank > 0 {
            out.push('\n');
        }
        blank = 0;
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Plain text: hard wraps kept, anything that looks like markdown syntax still
/// means what it says (a `>` quote, a `-` list), which is close enough.
pub fn text_to_md(text: &str) -> String {
    tidy(text)
}

/// The words of some markdown, with the syntax gone, on one line.
pub fn plain(md: &str, max: usize) -> String {
    let lines: Vec<Vec<char>> = md.lines().map(|l| l.chars().collect()).collect();
    let blocks = classify(&lines);
    let mut out = String::new();
    for (chars, block) in lines.iter().zip(blocks) {
        if matches!(block, Block::FenceOpen | Block::FenceClose) {
            continue;
        }
        let styled = style_line(chars, block);
        if styled.rule {
            continue;
        }
        for (c, cell) in chars.iter().zip(&styled.cells) {
            if !cell.hidden && cell.repl.is_none() {
                out.push(*c);
            }
        }
        out.push(' ');
        if out.len() > max * 2 {
            break;
        }
    }
    // Bare addresses say nothing in a one-line preview.
    let words: Vec<&str> = out.split_whitespace().filter(|w| !w.starts_with("http://") && !w.starts_with("https://")).collect();
    let mut joined = words.join(" ");
    if joined.chars().count() > max {
        let cut: String = joined.chars().take(max).collect();
        // At a word boundary, unless that throws away most of it.
        let at = cut.rfind(' ').filter(|&at| at > cut.len() / 2).unwrap_or(cut.len());
        joined = cut[..at].trim_end().to_string() + "…";
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_mail_becomes_readable_markdown() {
        let html = r#"<html><head><style>p{color:red}</style></head><body>
            <span style="display:none">&#847;&zwnj;&nbsp;&#847;&zwnj;&nbsp;</span>
            <table><tr><td><h1>Hello</h1></td></tr><tr><td><p>Read <a href="https://x.y/a">the <b>post</b></a>.</p>
            <img src="https://track.er/pixel.gif" width="1" height="1"></td></tr></table>
            <script>alert(1)</script></body></html>"#;
        let md = html_to_md(html);
        assert!(md.contains("# Hello"), "{md}");
        assert!(md.contains("[the **post**](https://x.y/a)"), "{md}");
        assert!(!md.contains("pixel") && !md.contains("color:red") && !md.contains("alert"), "{md}");
        assert!(!md.contains('|'), "layout tables are flattened: {md}");
        assert!(!md.contains("\n\n\n"));
    }

    #[test]
    fn plain_strips_the_syntax() {
        assert_eq!(plain("# Hi\n\nSome **bold** [link](https://a.b) text\n- item", 100), "Hi Some bold link text item");
        assert_eq!(plain("one two three four", 9), "one two…");
        assert_eq!(plain("Article URL: https://a.b/c Points: 5", 100), "Article URL: Points: 5");
    }

    #[test]
    fn tidy_drops_padding_and_collapses_blanks() {
        assert_eq!(tidy("a\u{a0}b\n\n\n\u{200c}\u{a0}\n\nc\r\n"), "a b\n\nc\n");
    }
}
