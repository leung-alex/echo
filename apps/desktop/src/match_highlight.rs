//! Presentation-only match formatting. Source payload and accessible names stay
//! plain text. Markup metacharacters are escaped before applying trusted styling.
use echo_engine::FuzzyMatcher;
use slint::StyledText;
use std::ops::Range;
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            ' ' => out.push_str("&#32;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("  \n"),
            '\r' => {}
            c if c.is_ascii_punctuation() => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}
fn markup(text: &str, ranges: &[Range<usize>], color: &str) -> String {
    let mut result = String::new();
    let mut end = 0;
    for range in ranges {
        if range.start < end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            continue;
        }
        result.push_str(&escape(&text[end..range.start]));
        let matched = escape(&text[range.clone()]);
        if color.is_empty() {
            result.push_str(&format!("**{matched}**"));
        } else {
            result.push_str(&format!("<font color='{color}'>{matched}</font>"));
        }
        end = range.end;
    }
    result.push_str(&escape(&text[end..]));
    result
}
fn styled(text: &str, matcher: &mut FuzzyMatcher, color: &str) -> (StyledText, Vec<Range<usize>>) {
    let ranges = matcher.highlights(text);
    if ranges.is_empty() {
        return (StyledText::from_plain_text(text), ranges);
    }
    let value = StyledText::from_markdown(&markup(text, &ranges, color))
        .unwrap_or_else(|_| StyledText::from_plain_text(text));
    (value, ranges)
}
fn ranges(matches: Vec<Range<usize>>) -> slint::ModelRc<crate::MatchRange> {
    if matches.is_empty() {
        return Default::default();
    }
    std::rc::Rc::new(slint::VecModel::from(
        matches
            .into_iter()
            .map(|r| crate::MatchRange {
                start: r.start as i32,
                end: r.end as i32,
            })
            .collect::<Vec<_>>(),
    ))
    .into()
}
pub fn apply(row: &mut crate::EntryRow, matcher: &mut FuzzyMatcher, color: &str) {
    let (title, a) = styled(row.title.as_str(), matcher, color);
    let (body, b) = styled(row.body.as_str(), matcher, color);
    let (tags, c) = styled(row.tags.as_str(), matcher, color);
    row.match_count = (a.len() + b.len() + c.len()) as i32;
    row.title_matches = ranges(a);
    row.body_matches = ranges(b);
    row.tags_matches = ranges(c);
    row.title_rich = title;
    row.body_rich = body;
    row.tags_rich = tags;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_text_color_and_alpha_are_preserved() {
        let bold = markup("match", &[0..5], "");
        assert_eq!(bold, "**match**");
        let alpha = markup("match", &[0..5], "#12345680");
        assert!(StyledText::from_markdown(&alpha).is_ok());
        assert_ne!(
            StyledText::from_markdown(&alpha).unwrap(),
            StyledText::from_markdown(&markup("match", &[0..5], "#123456ff")).unwrap()
        );
    }
    #[test]
    fn markup_escapes_content_not_instructions() {
        let text = "<script> ** [x](url) & \\";
        let marked = markup(text, &[0..1], "#855400");
        assert!(!marked.contains("<script>"));
        assert!(marked.contains("\\<"));
        assert!(StyledText::from_markdown(&marked).is_ok());
    }
    #[test]
    fn fuzzy_matches_are_colored_without_changing_font_weight() {
        let text = "Worktrees/echo/ui";
        let ranges = FuzzyMatcher::new("wrk ui").highlights(text);
        let value = markup(text, &ranges, "#855400");
        assert!(value.contains("<font"));
        assert!(value.contains(">W</font>"));
        assert!(StyledText::from_markdown(&value).is_ok());
    }
    #[test]
    fn unicode_highlights_keep_graphemes_and_escape_neighboring_markup() {
        let text = "中文 👨‍👩‍👧‍👦 Cafe\u{301} <script> ** [x](url) & \\";
        let ranges = FuzzyMatcher::new("👨‍👩‍👧‍👦 cafe 中文").highlights(text);
        let value = markup(text, &ranges, "#855400");
        assert!(value.contains(">👨‍👩‍👧‍👦</font>"));
        assert!(value.contains(">Cafe\u{301}</font>"));
        assert!(value.contains(">中文</font>"));
        assert!(!value.contains("<script>"));
        assert!(value.contains("\\<script\\>"));
        assert!(StyledText::from_markdown(&value).is_ok());
    }
}
