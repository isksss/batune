//! Links under the pointer: OSC 8 hyperlinks, or URLs found in the text.

use crate::vt::{Cell, Flags, Term};

use crate::selection::{self, Point, Range};

/// Schemes that are opened and detected. Others (`file:`, `ssh:`, custom
/// handlers) are left alone: an OSC 8 link can come from any program, and
/// handing an arbitrary URI to the desktop could run something.
const SCHEMES: [&str; 3] = ["https://", "http://", "mailto:"];

/// Characters that end a detected URL besides whitespace and wide
/// characters.
const URL_DELIMITERS: &str = "<>\"'`{}|\\^";

/// Punctuation that ends a sentence rather than the URL in front of it.
const TRAILING: &str = ".,:;!?";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub uri: String,
    /// The cells drawn underlined while hovered.
    pub range: Range,
}

fn openable(uri: &str) -> bool {
    SCHEMES.iter().any(|s| {
        uri.get(..s.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(s))
    })
}

/// The link covering `at`: an OSC 8 hyperlink if the cell has one, else a
/// URL in the text of the line. Lines joined by auto-wrap count as one.
pub fn at(term: &Term, at: Point) -> Option<Link> {
    let cells = logical_line(term, at.line);
    let index = cells.iter().position(|(p, _)| *p == at)?;
    if let Some(link) = cells[index].1.hyperlink() {
        if !openable(&link.uri) {
            return None;
        }
        let same = |(_, c): &(Point, &Cell)| c.hyperlink() == Some(link);
        let start = cells[..index]
            .iter()
            .rposition(|c| !same(c))
            .map_or(0, |i| i + 1);
        let end = cells[index..]
            .iter()
            .position(|c| !same(c))
            .map_or(cells.len(), |i| index + i);
        return Some(Link {
            uri: link.uri.clone(),
            range: Range {
                start: cells[start].0,
                end: cells[end - 1].0,
            },
        });
    }
    let chars: Vec<(Point, &Cell)> = cells
        .into_iter()
        .filter(|(_, c)| !c.flags.contains(Flags::WIDE_SPACER))
        .collect();
    let text: Vec<char> = chars.iter().map(|(_, c)| c.c).collect();
    find_urls(&text).into_iter().find_map(|(start, end)| {
        let last = chars[end - 1];
        let range = Range {
            start: chars[start].0,
            end: if last.1.flags.contains(Flags::WIDE) {
                Point::new(last.0.line, last.0.col + 1)
            } else {
                last.0
            },
        };
        range.contains(at).then(|| Link {
            uri: text[start..end].iter().collect(),
            range,
        })
    })
}

/// Rows walked each way from the pointer's row when joining wrapped rows.
/// shortcut: a link longer than this many rows is cut, raise it if one is.
const MAX_WRAPPED_ROWS: usize = 64;

/// Every cell of the rows joined by auto-wrap with `line`, in order, up to
/// [`MAX_WRAPPED_ROWS`] each way: this runs on every pointer move while Ctrl
/// is held, and a scrollback of one wrapped line would be read whole.
fn logical_line(term: &Term, line: usize) -> Vec<(Point, &Cell)> {
    let mut first = line;
    while first > 0 && line - first < MAX_WRAPPED_ROWS && selection::row(term, first - 1).wrapped()
    {
        first -= 1;
    }
    let mut last = line;
    let count = selection::line_count(term);
    while last + 1 < count && last - line < MAX_WRAPPED_ROWS && selection::row(term, last).wrapped()
    {
        last += 1;
    }
    (first..=last)
        .flat_map(|l| {
            let cells = selection::row(term, l).cells();
            cells
                .iter()
                .enumerate()
                .map(move |(col, c)| (Point::new(l, col), c))
        })
        .collect()
}

/// Char index ranges `start..end` of the URLs in `text`.
fn find_urls(text: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let scheme = SCHEMES.iter().find(|s| starts_with(&text[i..], s));
        // ASCII only: Japanese text often runs straight into a URL.
        let boundary = i == 0 || !text[i - 1].is_ascii_alphanumeric();
        let Some(scheme) = scheme.filter(|_| boundary) else {
            i += 1;
            continue;
        };
        let body = i + scheme.len();
        let mut end = body;
        while end < text.len() && is_url_char(text[end]) {
            end += 1;
        }
        end = trim_end(&text[i..end]) + i;
        if end > body {
            out.push((i, end));
            i = end;
        } else {
            i = body;
        }
    }
    out
}

fn starts_with(text: &[char], prefix: &str) -> bool {
    let mut chars = text.iter();
    prefix
        .chars()
        .all(|p| chars.next().is_some_and(|c| c.eq_ignore_ascii_case(&p)))
}

/// Wide characters end a URL so that Japanese text written right after
/// one ("…/docs を見て") is not swallowed; percent-encoded URLs are what
/// programs print anyway.
fn is_url_char(c: char) -> bool {
    !c.is_whitespace()
        && !c.is_control()
        && !URL_DELIMITERS.contains(c)
        && unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) == 1
}

/// Length of `url` without trailing sentence punctuation and without a
/// closing bracket that has no opening one in the URL (as in
/// "(see https://example.com)").
fn trim_end(url: &[char]) -> usize {
    // Counted once and updated as chars come off; recounting per char is
    // quadratic on a URL followed by many `)`.
    let count = |x: char| url.iter().filter(|&&c| c == x).count();
    let open = [count('('), count('[')];
    let mut close = [count(')'), count(']')];
    let mut len = url.len();
    while len > 0 {
        let c = url[len - 1];
        let bracket = [')', ']'].iter().position(|&b| b == c);
        if TRAILING.contains(c) || bracket.is_some_and(|b| open[b] < close[b]) {
            if let Some(b) = bracket {
                close[b] -= 1;
            }
            len -= 1;
        } else {
            break;
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        find_urls(&chars)
            .into_iter()
            .map(|(s, e)| chars[s..e].iter().collect())
            .collect()
    }

    #[test]
    fn urls_are_found_with_their_schemes() {
        assert_eq!(
            urls("see https://example.com/a?b=c#d and HTTP://x.org, mailto:me@example.com"),
            [
                "https://example.com/a?b=c#d",
                "HTTP://x.org",
                "mailto:me@example.com"
            ]
        );
    }

    #[test]
    fn other_schemes_and_bare_schemes_are_not_urls() {
        assert!(urls("file:///etc/passwd ftp://x javascript:alert(1)").is_empty());
        assert!(urls("https:// alone").is_empty());
        assert!(urls("xhttps://example.com").is_empty());
    }

    #[test]
    fn trailing_punctuation_and_unmatched_brackets_are_left_out() {
        assert_eq!(
            urls("(see https://example.com/x)."),
            ["https://example.com/x"]
        );
        assert_eq!(
            urls("https://en.wikipedia.org/wiki/Rust_(language)"),
            ["https://en.wikipedia.org/wiki/Rust_(language)"]
        );
        assert_eq!(urls("<https://example.com>"), ["https://example.com"]);
        assert_eq!(urls("\"https://example.com\""), ["https://example.com"]);
    }

    #[test]
    fn many_closing_brackets_after_a_url_are_trimmed_quickly() {
        let text = format!("https://a{}", ")".repeat(200_000));
        let start = std::time::Instant::now();
        assert_eq!(urls(&text), ["https://a"]);
        assert_eq!(
            urls(&format!("https://a({}", ")".repeat(5))),
            ["https://a()"]
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn a_long_wrapped_line_is_joined_only_near_the_pointer() {
        let t = term(10, 500, &"x".repeat(10 * 500));
        let cells = logical_line(&t, 250);
        assert_eq!(cells.first().unwrap().0.line, 250 - MAX_WRAPPED_ROWS);
        assert_eq!(cells.last().unwrap().0.line, 250 + MAX_WRAPPED_ROWS);
    }

    #[test]
    fn wide_text_ends_the_url() {
        assert_eq!(
            urls("詳細はhttps://example.com/docsを見て"),
            ["https://example.com/docs"]
        );
    }

    #[test]
    fn only_web_and_mail_links_are_opened() {
        assert!(openable("https://example.com"));
        assert!(openable("HTTP://example.com"));
        assert!(openable("mailto:me@example.com"));
        assert!(!openable("file:///etc/passwd"));
        assert!(!openable("javascript:alert(1)"));
        assert!(!openable("http"));
    }

    fn term(cols: usize, rows: usize, input: &str) -> Term {
        let mut term = Term::new(cols, rows);
        term.advance(input.as_bytes());
        term
    }

    #[test]
    fn a_url_is_found_from_any_of_its_cells() {
        let t = term(40, 2, "go to https://example.com now");
        let link = at(&t, Point::new(0, 10)).unwrap();
        assert_eq!(link.uri, "https://example.com");
        assert_eq!(
            link.range,
            Range {
                start: Point::new(0, 6),
                end: Point::new(0, 24)
            }
        );
        assert!(at(&t, Point::new(0, 2)).is_none());
        assert!(at(&t, Point::new(0, 25)).is_none());
    }

    #[test]
    fn a_url_wrapped_across_rows_is_one_link() {
        let t = term(10, 3, "x https://example.com/abc");
        let link = at(&t, Point::new(2, 1)).unwrap();
        assert_eq!(link.uri, "https://example.com/abc");
        assert_eq!(link.range.start, Point::new(0, 2));
        assert_eq!(link.range.end, Point::new(2, 4));
    }

    #[test]
    fn osc8_text_opens_its_target() {
        let t = term(
            40,
            1,
            "a \x1b]8;;https://example.com/\x1b\\click here\x1b]8;;\x1b\\ b",
        );
        let link = at(&t, Point::new(0, 5)).unwrap();
        assert_eq!(link.uri, "https://example.com/");
        assert_eq!(
            link.range,
            Range {
                start: Point::new(0, 2),
                end: Point::new(0, 11)
            }
        );
        assert!(at(&t, Point::new(0, 12)).is_none());
    }

    #[test]
    fn osc8_links_to_other_schemes_are_ignored() {
        let t = term(40, 1, "\x1b]8;;file:///etc/passwd\x1b\\text\x1b]8;;\x1b\\");
        assert!(at(&t, Point::new(0, 1)).is_none());
    }
}
