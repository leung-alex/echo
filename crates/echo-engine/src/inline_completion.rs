//! Verified, ephemeral replacement ranges. No platform handles, key-to-text guessing,
//! whole-field rewrite, or serialization of a composer's surrounding content.
use std::ops::Range;

pub const MAX_COMPOSER_UNITS: usize = 65_536;
pub const MAX_QUERY_UNITS: usize = 2_048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlineTicket {
    pub session: u64,
    pub revision: u64,
    pub input_serial: u64,
}

/// Offsets are UTF-16 code units, never UTF-8 bytes or UIA TextUnit counts.
/// This intentionally does not implement Debug/Serialize: surrounding text is private.
#[derive(Clone)]
pub struct ComposerSnapshot {
    pub text: Vec<u16>,
    pub selection: Range<usize>,
}
impl ComposerSnapshot {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.text.len() > MAX_COMPOSER_UNITS {
            return Err("Input is too large for a verified inline session");
        }
        if self.selection.start > self.selection.end
            || self.selection.end > self.text.len()
            || !utf16_boundary(&self.text, self.selection.start)
            || !utf16_boundary(&self.text, self.selection.end)
            || String::from_utf16(&self.text).is_err()
        {
            return Err("Input provider returned an invalid text selection");
        }
        Ok(())
    }
}
fn utf16_boundary(text: &[u16], at: usize) -> bool {
    at <= text.len()
        && !(at > 0
            && at < text.len()
            && (0xd800..=0xdbff).contains(&text[at - 1])
            && (0xdc00..=0xdfff).contains(&text[at]))
}

/// Immutable prefix/suffix limit a session to the exact query region. Backspace,
/// paste, Unicode and edits within the query work without counting key presses.
pub struct QueryRange {
    prefix: Vec<u16>,
    suffix: Vec<u16>,
    query: Vec<u16>,
    search_started: bool,
    revision: u64,
}
impl QueryRange {
    pub fn begin(snapshot: &ComposerSnapshot) -> Result<Self, &'static str> {
        snapshot.validate()?;
        let query = snapshot.text[snapshot.selection.clone()].to_vec();
        validate_query(&query)?;
        let search_started = snapshot.selection.is_empty();
        Ok(Self {
            prefix: snapshot.text[..snapshot.selection.start].to_vec(),
            suffix: snapshot.text[snapshot.selection.end..].to_vec(),
            query,
            search_started,
            revision: 1,
        })
    }
    /// Re-enter after the begin-time context proved to be generated
    /// decoration. A collapsed caret followed only by blanks or sentinels is
    /// the effective end of the editable text, so the trailing word before it
    /// is in-flight user input and resumes as the query.
    pub fn resume(snapshot: &ComposerSnapshot) -> Result<Self, &'static str> {
        snapshot.validate()?;
        let at_effective_end = snapshot.selection.is_empty()
            && snapshot.text[snapshot.selection.end..]
                .iter()
                .all(|unit| {
                    *unit == 0x200b
                        || char::from_u32(u32::from(*unit)).is_some_and(char::is_whitespace)
                });
        let (start, end) = if at_effective_end {
            trailing_token_range(&snapshot.text, snapshot.selection.end)
        } else {
            (snapshot.selection.start, snapshot.selection.end)
        };
        let query = snapshot.text[start..end].to_vec();
        validate_query(&query)?;
        Ok(Self {
            prefix: snapshot.text[..start].to_vec(),
            suffix: snapshot.text[end..].to_vec(),
            query,
            search_started: true,
            revision: 1,
        })
    }
    pub fn query(&self) -> String {
        if !self.search_started {
            return String::new();
        }
        // begin/observe validate UTF-16 and boundaries before accepting data.
        String::from_utf16(&self.query).unwrap_or_default()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn query_units(&self) -> &[u16] {
        &self.query
    }
    pub fn span(&self) -> Range<usize> {
        self.prefix.len()..self.prefix.len() + self.query.len()
    }
    /// Project uncommitted composition into a search query without changing the
    /// retained replacement span or revision. Only a later committed observation
    /// may update that span. The adapter proves the source and composition state.
    pub fn preview_composition(
        &self,
        snapshot: &ComposerSnapshot,
        preedit: &str,
    ) -> Result<String, &'static str> {
        let span = self.validate_context(snapshot)?;
        let mut query = snapshot.text[span.start..snapshot.selection.start].to_vec();
        query.extend(preedit.encode_utf16());
        query.extend_from_slice(&snapshot.text[snapshot.selection.end..span.end]);
        validate_query(&query)?;
        String::from_utf16(&query).map_err(|_| "Composition contains invalid text")
    }
    pub fn observe(&mut self, snapshot: &ComposerSnapshot) -> Result<bool, &'static str> {
        let span = self.validate_context(snapshot)?;
        let query = &snapshot.text[span];
        validate_query(query)?;
        let changed = query != self.query;
        if changed {
            self.query = query.to_vec();
            self.search_started = true;
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or("Inline revision exhausted")?;
        }
        Ok(changed)
    }
    fn validate_context(&self, snapshot: &ComposerSnapshot) -> Result<Range<usize>, &'static str> {
        snapshot.validate()?;
        let minimum = self
            .prefix
            .len()
            .checked_add(self.suffix.len())
            .ok_or("Invalid inline context")?;
        if snapshot.text.len() < minimum
            || !snapshot.text.starts_with(&self.prefix)
            || !snapshot.text.ends_with(&self.suffix)
        {
            return Err("Text outside the inline query changed; nothing was replaced");
        }
        let span = self.prefix.len()..snapshot.text.len() - self.suffix.len();
        if snapshot.selection.start < span.start || snapshot.selection.end > span.end {
            return Err("The caret left the inline query; nothing was replaced");
        }
        if !utf16_boundary(&snapshot.text, span.start) || !utf16_boundary(&snapshot.text, span.end)
        {
            return Err("Query boundary split a Unicode character");
        }
        Ok(span)
    }
    pub fn seal(
        &self,
        snapshot: &ComposerSnapshot,
        revision: u64,
    ) -> Result<Range<usize>, &'static str> {
        if revision != self.revision {
            return Err("Results belong to an older query");
        }
        let span = self.validate_context(snapshot)?;
        if snapshot.text[span.clone()] != self.query {
            return Err("The query changed before replacement; nothing was replaced");
        }
        Ok(span)
    }
    pub fn matches_replacement(&self, actual: &[u16], inserted: &[u16]) -> bool {
        actual.len() == self.prefix.len() + inserted.len() + self.suffix.len()
            && actual.starts_with(&self.prefix)
            && actual.ends_with(&self.suffix)
            && &actual[self.prefix.len()..self.prefix.len() + inserted.len()] == inserted
    }
    /// Same contract as `matches_replacement`, but units the adapter proves to
    /// be host-generated structure (sentinels, decoration markers) carry no
    /// content and are dropped on both sides before the exact comparison.
    /// Significant text must still match verbatim, including order.
    pub fn matches_replacement_except(
        &self,
        actual: &[u16],
        inserted: &[u16],
        ignored: impl Fn(u16) -> bool,
    ) -> bool {
        let strip = |units: &[u16]| {
            units
                .iter()
                .copied()
                .filter(|unit| !ignored(*unit))
                .collect::<Vec<u16>>()
        };
        let mut expected = strip(&self.prefix);
        expected.extend(strip(inserted));
        expected.extend(strip(&self.suffix));
        strip(actual) == expected
    }
}
fn trailing_token_range(text: &[u16], caret: usize) -> (usize, usize) {
    let Ok(value) = String::from_utf16(&text[..caret]) else {
        return (caret, caret);
    };
    let mut units = 0usize;
    let mut start = caret;
    for ch in value.chars().rev() {
        let width = ch.len_utf16();
        units = units.saturating_add(width);
        if ch.is_whitespace() {
            break;
        }
        start = caret.saturating_sub(units);
    }
    let token = &text[start..caret];
    if token
        .iter()
        .any(|unit| char::from_u32(u32::from(*unit)).is_some_and(char::is_alphanumeric))
    {
        (start, caret)
    } else {
        (caret, caret)
    }
}
fn validate_query(query: &[u16]) -> Result<(), &'static str> {
    if query.len() > MAX_QUERY_UNITS {
        return Err("Inline query is too long");
    }
    if query.iter().any(|u| *u == 0 || *u == 0xfffc) {
        return Err("Inline query contains an embedded object, not plain search text");
    }
    if String::from_utf16(query).is_err() {
        return Err("Invalid Unicode query");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composition_preview_preserves_the_committed_ticket_and_range() {
        let mut query = QueryRange::begin(&snap("pre||post", 4, 4)).unwrap();
        assert_eq!(
            query
                .preview_composition(&snap("pre||post", 4, 4), "nihao")
                .unwrap(),
            "nihao"
        );
        assert_eq!(query.query(), "");
        assert_eq!(query.revision(), 1);
        assert_eq!(query.span(), 4..4);
        query.observe(&snap("pre|你好|post", 6, 6)).unwrap();
        assert_eq!(query.query(), "你好");
        assert_eq!(query.seal(&snap("pre|你好|post", 6, 6), 2).unwrap(), 4..6);
    }
    #[test]
    fn composition_preview_uses_current_selection_and_rejects_context_escape() {
        let mut query = QueryRange::begin(&snap("前后", 1, 1)).unwrap();
        query.observe(&snap("前abc后", 4, 4)).unwrap();
        assert_eq!(
            query
                .preview_composition(&snap("前abc后", 2, 3), "🙂")
                .unwrap(),
            "a🙂c"
        );
        assert!(query
            .preview_composition(&snap("前abc后", 0, 1), "ni")
            .is_err());
        assert!(query
            .preview_composition(&snap("外abc后", 2, 2), "ni")
            .is_err());
        assert!(query
            .preview_composition(&snap("前abc后", 2, 2), "\0")
            .is_err());
        assert!(query
            .preview_composition(&snap("前abc后", 2, 2), &"x".repeat(MAX_QUERY_UNITS))
            .is_err());
        assert_eq!(query.query(), "abc");
    }
    fn snap(text: &str, start: usize, end: usize) -> ComposerSnapshot {
        ComposerSnapshot {
            text: text.encode_utf16().collect(),
            selection: start..end,
        }
    }
    #[test]
    fn empty_query_only_replaces_the_new_segment_not_the_composer() {
        let mut q = QueryRange::begin(&snap("prefix: suffix", 8, 8)).unwrap();
        q.observe(&snap("prefix: emailsuffix", 13, 13)).unwrap();
        assert_eq!(q.query(), "email");
        assert_eq!(
            q.seal(&snap("prefix: emailsuffix", 13, 13), q.revision())
                .unwrap(),
            8..13
        );
        assert!(q.matches_replacement(
            &"prefix: a@b.testsuffix".encode_utf16().collect::<Vec<_>>(),
            &"a@b.test".encode_utf16().collect::<Vec<_>>()
        ));
    }
    #[test]
    fn collapsed_caret_never_seeds_a_replacement_range() {
        // Opening on existing text starts an empty query; only the selection
        // itself is ever a replacement span.
        let q = QueryRange::begin(&snap("prefix: git", 11, 11)).unwrap();
        assert_eq!(q.query(), "");
        assert_eq!(q.span(), 11..11);
        let q = QueryRange::begin(&snap("prefix: git\n", 11, 11)).unwrap();
        assert_eq!(q.query(), "");
        assert_eq!(q.span(), 11..11);
    }
    #[test]
    fn collapsed_caret_inside_text_does_not_guess_a_replacement_range() {
        let q = QueryRange::begin(&snap("prefixgit", 6, 6)).unwrap();
        assert_eq!(q.query(), "");
        assert_eq!(q.span(), 6..6);
    }
    #[test]
    fn resume_before_blank_tail_seeds_the_trailing_token() {
        // Providers may append a paragraph mark or decoration sentinel after
        // the editable text; a caret in front of it is still at the end.
        let mut q = QueryRange::resume(&snap("prefix: git\n", 11, 11)).unwrap();
        assert_eq!(q.query(), "git");
        assert_eq!(q.span(), 8..11);
        q.observe(&snap("prefix: github\n", 14, 14)).unwrap();
        assert_eq!(q.query(), "github");
        assert_eq!(
            q.seal(&snap("prefix: github\n", 14, 14), q.revision())
                .unwrap(),
            8..14
        );
        let q = QueryRange::resume(&snap("git\u{200b}", 3, 3)).unwrap();
        assert_eq!(q.query(), "git");
        assert_eq!(q.span(), 0..3);
    }
    #[test]
    fn resume_before_nonblank_tail_stays_mid_text() {
        let q = QueryRange::resume(&snap("prefix: git tail", 11, 11)).unwrap();
        assert_eq!(q.query(), "");
        assert_eq!(q.span(), 11..11);
    }
    #[test]
    fn vanished_placeholder_context_can_be_rebound() {
        // The adapter drops a stored context that vanished before the first
        // real observation (it was decoration, not user text) and resumes on
        // the live snapshot.
        let mut q = QueryRange::begin(&snap("Ask anything\n", 11, 11)).unwrap();
        assert_eq!(q.revision(), 1);
        assert!(q.observe(&snap("g", 1, 1)).is_err());
        let rebound = QueryRange::resume(&snap("g", 1, 1)).unwrap();
        assert_eq!(rebound.query(), "g");
        assert_eq!(rebound.span(), 0..1);
    }
    #[test]
    fn preexisting_selection_is_only_a_replacement_range() {
        let initial = snap("left email right", 5, 10);
        let mut q = QueryRange::begin(&initial).unwrap();
        assert_eq!(q.query(), "");
        assert_eq!(q.span(), 5..10);
        assert_eq!(q.seal(&initial, q.revision()).unwrap(), 5..10);
        q.observe(&initial).unwrap();
        assert_eq!(
            q.query(),
            "",
            "unchanged provider notifications must not seed search"
        );
        q.observe(&snap("left new right", 8, 8)).unwrap();
        assert_eq!(q.query(), "new");
        assert_eq!(q.span(), 5..8);
        assert!(q.matches_replacement(
            &"left result right".encode_utf16().collect::<Vec<_>>(),
            &"result".encode_utf16().collect::<Vec<_>>()
        ));
    }
    #[test]
    fn actual_unicode_text_not_key_counts_drives_the_query() {
        let mut q = QueryRange::begin(&snap("前🙂后", 3, 3)).unwrap();
        q.observe(&snap("前🙂邮箱👨‍👩‍👧后", 13, 13)).unwrap();
        assert_eq!(q.query(), "邮箱👨‍👩‍👧");
        q.observe(&snap("前🙂邮后", 4, 4)).unwrap();
        assert_eq!(q.query(), "邮");
        q.observe(&snap("前🙂后", 3, 3)).unwrap();
        assert_eq!(q.query(), "");
    }
    #[test]
    fn cursor_and_selection_may_move_within_query_not_outside_it() {
        let mut q = QueryRange::begin(&snap("prepost", 3, 3)).unwrap();
        q.observe(&snap("prehello post", 9, 9)).unwrap();
        assert!(!q.observe(&snap("prehello post", 4, 7)).unwrap());
        assert!(q.observe(&snap("prehello post", 0, 0)).is_err());
    }
    #[test]
    fn deleting_prefix_or_suffix_never_expands_replacement_scope() {
        let q = QueryRange::begin(&snap("prefixsuffix", 6, 6)).unwrap();
        assert!(q.seal(&snap("prefisuffix", 5, 5), 1).is_err());
        assert!(q.seal(&snap("prefixuff ix", 6, 6), 1).is_err());
    }
    #[test]
    fn revision_and_current_text_are_both_checked() {
        let mut q = QueryRange::begin(&snap("", 0, 0)).unwrap();
        q.observe(&snap("new", 3, 3)).unwrap();
        assert!(q.seal(&snap("new", 3, 3), 1).is_err());
        assert!(q.seal(&snap("newer", 5, 5), 2).is_err());
        assert_eq!(q.seal(&snap("new", 3, 3), 2).unwrap(), 0..3);
    }
    #[test]
    fn repeated_substrings_do_not_confuse_the_owned_span() {
        let mut q = QueryRange::begin(&snap("email email", 6, 6)).unwrap();
        q.observe(&snap("email emailemail", 11, 11)).unwrap();
        assert_eq!(q.span(), 6..11);
    }
    #[test]
    fn multiline_query_is_valid_but_embedded_object_is_not() {
        let mut q = QueryRange::begin(&snap("", 0, 0)).unwrap();
        assert!(q.observe(&snap("a\r\nb", 4, 4)).is_ok());
        assert!(q.observe(&snap("\u{fffc}", 1, 1)).is_err());
    }
    #[test]
    fn invalid_utf16_and_split_surrogates_fail_closed() {
        assert!(ComposerSnapshot {
            text: vec![0xd800],
            selection: 0..0
        }
        .validate()
        .is_err());
        assert!(snap("🙂", 1, 1).validate().is_err());
        assert!(snap("a", 2, 2).validate().is_err());
        assert!(QueryRange::begin(&snap(
            &"a".repeat(MAX_QUERY_UNITS + 1),
            0,
            MAX_QUERY_UNITS + 1
        ))
        .is_err());
    }
}
