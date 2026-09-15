//! FZF-style, multi-word subsequence matching with nucleo. No platform/UI types.
//! Search scans complete scoped metadata, not the currently visible result page.
use crate::{PageCursor, QuickInsertItem, QuickInsertPage, SpaceId, SpaceStore, MAX_PAGE_SIZE};
use nucleo_matcher::{
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    Matcher, Utf32Str,
};
use sha2::{Digest, Sha256};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, VecDeque},
    ops::Range,
};
use unicode_segmentation::UnicodeSegmentation;

pub struct FuzzyMatcher {
    pattern: Pattern,
    matcher: Matcher,
    buffer: Vec<char>,
    indices: Vec<u32>,
}
impl FuzzyMatcher {
    pub fn new(query: &str) -> Self {
        // Contenteditable frequently turns a trailing space into NBSP. The query
        // syntax treats every Unicode whitespace as a separator, never a commit.
        let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
        Self {
            pattern: Pattern::new(
                &query,
                // Ordinary clipboard searches never opt into case sensitivity
                // just because the query contains an uppercase letter. Admission,
                // ranking and highlight ranges share this same pattern.
                CaseMatching::Ignore,
                Normalization::Smart,
                AtomKind::Fuzzy,
            ),
            matcher: Matcher::default(),
            buffer: Vec::new(),
            indices: Vec::new(),
        }
    }
    pub fn score(&mut self, text: &str) -> Option<u32> {
        self.pattern
            .score(Utf32Str::new(text, &mut self.buffer), &mut self.matcher)
    }
    /// Byte ranges of matching graphemes. Each field may contain only some query
    /// words; admission/ranking always checks all words against the full document.
    pub fn highlights(&mut self, text: &str) -> Vec<Range<usize>> {
        self.indices.clear();
        let haystack = Utf32Str::new(text, &mut self.buffer);
        for atom in &self.pattern.atoms {
            let start = self.indices.len();
            if atom
                .indices(haystack, &mut self.matcher, &mut self.indices)
                .is_none()
            {
                self.indices.truncate(start);
            }
        }
        self.indices.sort_unstable();
        self.indices.dedup();
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let mut at = 0;
        for (i, (offset, grapheme)) in text.grapheme_indices(true).enumerate() {
            while at < self.indices.len() && (self.indices[at] as usize) < i {
                at += 1;
            }
            if self.indices.get(at).copied() == Some(i as u32) {
                let end = offset + grapheme.len();
                if let Some(last) = ranges.last_mut().filter(|r| r.end == offset) {
                    last.end = end;
                } else {
                    ranges.push(offset..end);
                }
            }
        }
        ranges
    }
}
struct Candidate {
    item: QuickInsertItem,
    text: String,
}
struct Corpus {
    space: SpaceId,
    revision: i64,
    count: u64,
    bytes: usize,
    items: Vec<Candidate>,
}
struct CachedPage {
    space: SpaceId,
    revision: i64,
    count: u64,
    query: String,
    limit: u32,
    cursor: Option<PageCursor>,
    value: (QuickInsertPage, i64, u64),
}
#[derive(Default)]
pub(crate) struct FuzzySearchCache {
    corpora: VecDeque<Corpus>,
    last_page: Option<CachedPage>,
}
const CACHE_BYTES: usize = 4 * 1024 * 1024;
impl FuzzySearchCache {
    pub(crate) fn clear_results(&mut self) {
        self.last_page = None;
    }
    pub(crate) fn clear(&mut self) {
        self.corpora.clear();
        self.last_page = None;
    }
    pub(crate) fn search<S: SpaceStore>(
        &mut self,
        store: &S,
        space: SpaceId,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(QuickInsertPage, i64, u64), String> {
        if cancelled() {
            return Err("Search cancelled by newer work".into());
        }
        if query.len() > 16 * 1024 {
            return Err("Search query exceeds the bounded input limit".into());
        }
        let metadata = store
            .list_spaces()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|s| s.id == space)
            .ok_or("Space is unavailable")?;
        let normalized = query.split_whitespace().collect::<Vec<_>>().join(" ");
        let limit = limit.clamp(1, MAX_PAGE_SIZE);
        if let Some(cached) = &self.last_page {
            if cached.space == space
                && cached.revision == metadata.revision
                && cached.count == metadata.item_count
                && cached.query == normalized
                && cached.limit == limit
                && cached.cursor == cursor
                && !cancelled()
            {
                return Ok(cached.value.clone());
            }
        }
        self.last_page = None;
        let digest = Sha256::digest(normalized.as_bytes());
        let hash = u64::from_le_bytes(digest[..8].try_into().unwrap());
        let after = match cursor {
            None => None,
            Some(PageCursor::Fuzzy {
                space_id,
                revision,
                query_hash,
                score,
                order,
            }) if space_id == space.0 && revision == metadata.revision && query_hash == hash => {
                Some((Reverse(score), order))
            }
            _ => return Err("Fuzzy cursor is outdated or belongs to another query".into()),
        };
        let page_limit = limit;
        let limit = limit as usize;
        let mut matcher = FuzzyMatcher::new(&normalized);
        let mut ranked: BTreeMap<(Reverse<u32>, u64), QuickInsertItem> = BTreeMap::new();
        let mut total = 0u64;
        let mut visit = |order: u64, c: &Candidate| {
            if let Some(score) = matcher.score(&c.text) {
                total += 1;
                let key = (Reverse(score), order);
                if after.is_some_and(|a| key <= a) {
                    return;
                }
                if ranked.len() < limit + 1
                    || ranked.last_key_value().is_some_and(|(last, _)| key < *last)
                {
                    ranked.insert(key, c.item.clone());
                    if ranked.len() > limit + 1 {
                        ranked.pop_last();
                    }
                }
            }
        };
        self.corpora.retain(|c| {
            c.space != space || (c.revision == metadata.revision && c.count == metadata.item_count)
        });
        if let Some(index) = self.corpora.iter().position(|c| c.space == space) {
            let corpus = self.corpora.remove(index).unwrap();
            for (i, candidate) in corpus.items.iter().enumerate() {
                if i % 64 == 0 && cancelled() {
                    self.corpora.push_front(corpus);
                    return Err("Search cancelled by newer work".into());
                }
                visit(i as u64, candidate);
            }
            self.corpora.push_front(corpus);
        } else {
            let mut items = Vec::new();
            let mut bytes = std::mem::size_of::<Corpus>();
            let mut cacheable = true;
            let mut order = 0u64;
            let mut page_cursor = None;
            loop {
                if cancelled() {
                    return Err("Search cancelled by newer work".into());
                }
                let (batch, next) = if space == SpaceId::HISTORY {
                    let page = store
                        .list_entries("", MAX_PAGE_SIZE, page_cursor)
                        .map_err(|e| e.to_string())?;
                    let items = page
                        .items
                        .into_iter()
                        .map(|entry| {
                            let body = entry
                                .searchable_text
                                .as_ref()
                                .or(entry.preview_text.as_ref())
                                .cloned()
                                .unwrap_or_default();
                            let item =
                                crate::quick_insert::to_item(crate::history::history_item(entry));
                            candidate(item, body)
                        })
                        .collect::<Vec<_>>();
                    (items, page.next_cursor)
                } else {
                    let page = store
                        .list_space_items(space, "", MAX_PAGE_SIZE, page_cursor)
                        .map_err(|e| e.to_string())?;
                    let items = page
                        .page
                        .items
                        .into_iter()
                        .map(|saved| {
                            let mut item = QuickInsertItem::from_saved(saved);
                            let body = item
                                .editable_text
                                .take()
                                .or_else(|| item.preview_text.clone())
                                .unwrap_or_default();
                            candidate(item, body)
                        })
                        .collect::<Vec<_>>();
                    (items, page.page.next_cursor)
                };
                for c in batch {
                    if order % 64 == 0 && cancelled() {
                        return Err("Search cancelled by newer work".into());
                    }
                    visit(order, &c);
                    order += 1;
                    if cacheable {
                        bytes = bytes.saturating_add(
                            c.text.capacity() + std::mem::size_of::<String>() + c.item.held_bytes(),
                        );
                        if bytes <= CACHE_BYTES {
                            items.push(c);
                            let held = bytes
                                + (items.capacity() - items.len())
                                    * std::mem::size_of::<Candidate>();
                            if held > CACHE_BYTES {
                                cacheable = false;
                                items = Vec::new();
                            }
                        } else {
                            cacheable = false;
                            items = Vec::new();
                        }
                    }
                }
                if next.is_none() {
                    break;
                }
                if next == page_cursor {
                    return Err("Metadata pagination did not advance".into());
                }
                page_cursor = next;
            }
            // Do not mix pages from a mutation that raced a full corpus scan.
            let current = store
                .list_spaces()
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|s| s.id == space)
                .ok_or("Space was removed")?;
            if current.revision != metadata.revision || current.item_count != metadata.item_count {
                return Err("Search corpus is outdated; retry the current query".into());
            }
            if cacheable {
                bytes += (items.capacity() - items.len()) * std::mem::size_of::<Candidate>();
                while self.corpora.len() >= 3
                    || self.corpora.iter().map(|c| c.bytes).sum::<usize>() + bytes > CACHE_BYTES
                {
                    if self.corpora.pop_back().is_none() {
                        break;
                    }
                }
                self.corpora.push_front(Corpus {
                    space,
                    revision: metadata.revision,
                    count: metadata.item_count,
                    bytes,
                    items,
                });
            }
        }
        let more = ranked.len() > limit;
        if cancelled() {
            return Err("Search cancelled by newer work".into());
        }
        let mut values = ranked.into_iter().take(limit).collect::<Vec<_>>();
        let next_cursor = if more {
            values
                .last()
                .map(|((Reverse(score), order), _)| PageCursor::Fuzzy {
                    space_id: space.0,
                    revision: metadata.revision,
                    query_hash: hash,
                    score: *score,
                    order: *order,
                })
        } else {
            None
        };
        let value = (
            QuickInsertPage {
                items: values.drain(..).map(|(_, i)| i).collect(),
                next_cursor,
            },
            metadata.revision,
            total,
        );
        // At most one bounded result page; never cache an unbounded result set.
        let page_bytes = value
            .0
            .items
            .iter()
            .map(QuickInsertItem::held_bytes)
            .sum::<usize>()
            + (value.0.items.capacity() - value.0.items.len())
                * std::mem::size_of::<QuickInsertItem>()
            + normalized.capacity()
            + std::mem::size_of::<CachedPage>();
        self.last_page = if page_bytes <= 256 * 1024 {
            while self.corpora.iter().map(|c| c.bytes).sum::<usize>() + page_bytes > CACHE_BYTES {
                if self.corpora.pop_back().is_none() {
                    break;
                }
            }
            Some(CachedPage {
                space,
                revision: metadata.revision,
                count: metadata.item_count,
                query: normalized,
                limit: page_limit,
                cursor,
                value: value.clone(),
            })
        } else {
            None
        };
        Ok(value)
    }
}
fn candidate(item: QuickInsertItem, body: String) -> Candidate {
    let text = format!(
        "{}\n{}\n{}\n{}",
        item.name.as_deref().unwrap_or(""),
        body,
        item.tags.join(" "),
        item.source_app.as_deref().unwrap_or("")
    );
    Candidate { item, text }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retiring_query_keeps_bounded_corpus_until_explicit_trim() {
        let mut cache = FuzzySearchCache::default();
        cache.corpora.push_back(Corpus {
            space: SpaceId::FAVORITES,
            revision: 1,
            count: 0,
            bytes: 0,
            items: Vec::new(),
        });
        cache.last_page = Some(CachedPage {
            space: SpaceId::FAVORITES,
            revision: 1,
            count: 0,
            query: "synthetic query".into(),
            limit: 50,
            cursor: None,
            value: (
                QuickInsertPage {
                    items: Vec::new(),
                    next_cursor: None,
                },
                1,
                0,
            ),
        });
        cache.clear_results();
        assert!(cache.last_page.is_none());
        assert_eq!(cache.corpora.len(), 1);
        cache.clear();
        assert!(cache.corpora.is_empty());
    }
    #[test]
    fn space_separated_tokens_match_independently_and_fuzzily() {
        let mut m = FuzzyMatcher::new("wrk tr ui");
        assert!(m.score(r"D:\Worktrees\echo\ui").is_some());
        assert!(m.score("work unrelated").is_none());
        assert!(FuzzyMatcher::new("ui wrk")
            .score(r"D:\Worktrees\echo\ui")
            .is_some());
        assert_eq!(
            FuzzyMatcher::new("work ").score("worktree"),
            FuzzyMatcher::new("work").score("worktree")
        );
    }
    #[test]
    fn nonbreaking_space_and_multiple_spaces_are_query_separators() {
        let mut m = FuzzyMatcher::new(" wrk\u{a0}  echo  ");
        assert!(m.score("Worktrees/echo/ui").is_some());
        assert!(m.score("Worktrees/aster/ui").is_none());
    }
    #[test]
    fn highlight_indices_are_graphemes_not_utf8_or_utf16() {
        let text = "前🙂Cafe\u{301} 邮箱";
        let mut m = FuzzyMatcher::new("cafe 邮");
        let highlighted = m
            .highlights(text)
            .iter()
            .map(|r| &text[r.clone()])
            .collect::<String>();
        assert_eq!(highlighted, "Cafe\u{301}邮");
        assert!(m.score(text).is_some());
    }
    #[test]
    fn consecutive_and_boundary_matches_outscore_scattered_letters() {
        let mut m = FuzzyMatcher::new("work");
        assert!(m.score("work item").unwrap() > m.score("w___o___r___k item").unwrap());
    }
    #[test]
    fn query_case_does_not_change_matches_scores_or_highlights() {
        for (text, queries) in [
            ("最高睿频 ≥5.4GHz", ["hz", "HZ", "Hz", "hZ"]),
            ("work item", ["work", "WORK", "Work", "wOrK"]),
            ("WORK item", ["work", "WORK", "Work", "wOrK"]),
            ("中文 CPU 5.4GHz", ["cpu hz", "CPU HZ", "Cpu Hz", "cPu hZ"]),
        ] {
            let mut baseline = FuzzyMatcher::new(queries[0]);
            let score = baseline.score(text);
            let ranges = baseline.highlights(text);
            assert!(score.is_some());
            assert!(!ranges.is_empty());
            for query in queries {
                let mut matcher = FuzzyMatcher::new(query);
                assert_eq!(matcher.score(text), score, "query={query}, text={text}");
                assert_eq!(
                    matcher.highlights(text),
                    ranges,
                    "query={query}, text={text}"
                );
                assert!(matcher.score("unrelated").is_none());
            }
        }
    }
    #[test]
    fn family_emoji_highlights_preserve_the_whole_grapheme() {
        let text = "前👨‍👩‍👧‍👦Cafe\u{301} 中文 <x>& [a](b)";
        let mut matcher = FuzzyMatcher::new("👨‍👩‍👧‍👦 cafe 中");
        assert!(matcher.score(text).is_some());
        let highlighted = matcher
            .highlights(text)
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<String>();
        assert_eq!(highlighted, "👨‍👩‍👧‍👦Cafe\u{301}中");
    }
}
