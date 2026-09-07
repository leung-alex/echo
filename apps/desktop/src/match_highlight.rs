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
        result.push_str(&format!(
            "<font color='{color}'>**{}**</font>",
            escape(&text[range.clone()])
        ));
        end = range.end;
    }
    result.push_str(&escape(&text[end..]));
    result
}
fn styled(text: &str, matcher: &mut FuzzyMatcher, dark: bool) -> (StyledText, i32) {
    let ranges = matcher.highlights(text);
    if ranges.is_empty() {
        return (StyledText::from_plain_text(text), 0);
    }
    let color = if dark { "#ffd26f" } else { "#855400" };
    let value = StyledText::from_markdown(&markup(text, &ranges, color))
        .unwrap_or_else(|_| StyledText::from_plain_text(text));
    (value, ranges.len() as i32)
}
pub fn apply(row: &mut crate::EntryRow, matcher: &mut FuzzyMatcher, dark: bool) {
    let (title, a) = styled(row.title.as_str(), matcher, dark);
    let (body, b) = styled(row.body.as_str(), matcher, dark);
    let (tags, c) = styled(row.tags.as_str(), matcher, dark);
    let (source, d) = styled(row.source_label.as_str(), matcher, dark);
    row.title_rich = title;
    row.body_rich = body;
    row.tags_rich = tags;
    row.source_rich = source;
    row.match_count = a + b + c + d;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markup_escapes_content_not_instructions() {
        let text = "<script> ** [x](url) & \\";
        let marked = markup(text, &[0..1], "#855400");
        assert!(!marked.contains("<script>"));
        assert!(marked.contains("\\<"));
        assert!(StyledText::from_markdown(&marked).is_ok());
    }
    #[test]
    fn fuzzy_matches_are_bold_and_colored_without_styling_whole_row() {
        let text = "Worktrees/echo/ui";
        let ranges = FuzzyMatcher::new("wrk ui").highlights(text);
        let value = markup(text, &ranges, "#855400");
        assert!(value.contains("<font"));
        assert!(value.contains("**W**"));
        assert!(StyledText::from_markdown(&value).is_ok());
    }
}
