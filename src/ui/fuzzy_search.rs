//! Fuzzy search functionality for Diamond TUIs
//!
//! Provides consistent fuzzy search behavior across `dm log` and `dm checkout`.

use nucleo_matcher::{Config, Matcher, Utf32String};
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph},
};

/// State for fuzzy search filtering in TUIs
pub struct SearchState {
    /// User's search query
    query: String,
    /// Indices of items matching the query (into original items vec)
    filtered_indices: Vec<usize>,
    /// Match scores for sorting (higher = better match)
    match_scores: Vec<u32>,
    /// Nucleo matcher instance (reused for performance)
    matcher: Matcher,
}

impl SearchState {
    /// Create new search state for a list of items
    pub fn new(total_items: usize) -> Self {
        Self {
            query: String::new(),
            filtered_indices: (0..total_items).collect(),
            match_scores: vec![100; total_items],
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// Get current search query
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Get filtered indices (maps visible index → original index)
    pub fn filtered_indices(&self) -> &[usize] {
        &self.filtered_indices
    }

    /// Push character to search query
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
    }

    /// Remove last character from search query
    pub fn pop_char(&mut self) {
        self.query.pop();
    }

    /// Clear search query
    pub fn clear(&mut self) {
        self.query.clear();
    }

    /// Check if search query is empty
    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
    }

    /// Filter items based on current query
    ///
    /// Generic over any type that implements `AsRef<str>` for the searchable field.
    pub fn filter<T, F>(&mut self, items: &[T], get_name: F)
    where
        F: Fn(&T) -> &str,
    {
        if self.query.is_empty() {
            // Empty query - show all items
            self.filtered_indices = (0..items.len()).collect();
            self.match_scores = vec![100; items.len()];
            return;
        }

        let needle = Utf32String::from(self.query.as_str());

        let mut results: Vec<(usize, u32)> = items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                let haystack = Utf32String::from(get_name(item));

                self.matcher
                    .fuzzy_match(haystack.slice(..), needle.slice(..))
                    .map(|score| (idx, score as u32))
            })
            .collect();

        // Sort by score descending (best matches first)
        results.sort_by(|a, b| b.1.cmp(&a.1));

        self.filtered_indices = results.iter().map(|(idx, _)| *idx).collect();
        self.match_scores = results.iter().map(|(_, score)| *score).collect();
    }

    /// Get match indices for highlighting a specific item
    pub fn get_match_indices(&mut self, text: &str) -> Vec<usize> {
        if self.query.is_empty() {
            return Vec::new();
        }

        let haystack = Utf32String::from(text);
        let needle = Utf32String::from(self.query.as_str());

        let mut indices = Vec::new();
        self.matcher
            .fuzzy_indices(haystack.slice(..), needle.slice(..), &mut indices);
        // Convert u32 indices to usize
        indices.into_iter().map(|i| i as usize).collect()
    }
}

/// Constant for "no matches" message
pub const NO_MATCHES_MESSAGE: &str = "  No branches match your search";

/// Render search input box (consistent across TUIs)
pub fn render_search_box(query: &str) -> Paragraph<'_> {
    let search_text = if query.is_empty() {
        "Type to search...".to_string()
    } else {
        format!("Search: {}", query)
    };

    let search_style = if query.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };

    Paragraph::new(search_text).style(search_style).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Search ")
            .title_style(Style::default().add_modifier(Modifier::BOLD)),
    )
}

/// Highlight matched characters in text
///
/// Returns a vector of Spans with matched characters highlighted in cyan.
pub fn highlight_matches<'a>(text: &'a str, match_indices: &[usize], base_style: Style) -> Vec<Span<'a>> {
    if match_indices.is_empty() {
        // No highlighting needed
        return vec![Span::styled(text, base_style)];
    }

    let mut spans = Vec::new();
    let mut last_idx = 0;

    for &match_idx in match_indices {
        // Non-matched chars before this match
        if match_idx > last_idx {
            let substr: String = text.chars().skip(last_idx).take(match_idx - last_idx).collect();
            spans.push(Span::styled(substr, base_style));
        }

        // Matched char (highlighted)
        if let Some(c) = text.chars().nth(match_idx) {
            spans.push(Span::styled(
                c.to_string(),
                base_style.fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
        }

        last_idx = match_idx + 1;
    }

    // Remaining non-matched chars
    if last_idx < text.len() {
        let substr: String = text.chars().skip(last_idx).collect();
        spans.push(Span::styled(substr, base_style));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_state_empty_query_shows_all() {
        let items = vec!["feature-1", "feature-2"];
        let mut search = SearchState::new(items.len());
        search.filter(&items, |s| s);

        assert_eq!(search.filtered_indices().len(), 2);
        assert_eq!(search.filtered_indices(), &[0, 1]);
    }

    #[test]
    fn test_search_state_filters_by_query() {
        let items = vec!["feature-auth", "feature-payment", "fix-authentication"];
        let mut search = SearchState::new(items.len());
        search.push_char('a');
        search.push_char('u');
        search.push_char('t');
        search.push_char('h');
        search.filter(&items, |s| s);

        // Should match feature-auth and fix-authentication
        assert_eq!(search.filtered_indices().len(), 2);
        assert!(search.filtered_indices().contains(&0)); // feature-auth
        assert!(search.filtered_indices().contains(&2)); // fix-authentication
    }

    #[test]
    fn test_search_state_fuzzy_matching() {
        let items = vec!["feature-authentication"];
        let mut search = SearchState::new(items.len());

        // Fuzzy match: "fauth" should match "feature-authentication"
        for c in "fauth".chars() {
            search.push_char(c);
        }
        search.filter(&items, |s| s);

        assert_eq!(search.filtered_indices().len(), 1);
    }

    #[test]
    fn test_search_state_case_insensitive() {
        let items = vec!["Feature-Auth"];
        let mut search = SearchState::new(items.len());
        for c in "feature".chars() {
            search.push_char(c);
        }
        search.filter(&items, |s| s);

        assert_eq!(search.filtered_indices().len(), 1);
    }

    #[test]
    fn test_search_state_no_matches() {
        let items = vec!["feature-1"];
        let mut search = SearchState::new(items.len());
        for c in "nonexistent".chars() {
            search.push_char(c);
        }
        search.filter(&items, |s| s);

        assert_eq!(search.filtered_indices().len(), 0);
    }

    #[test]
    fn test_search_state_pop_char() {
        let items = vec!["feature-1"];
        let mut search = SearchState::new(items.len());
        search.push_char('x');
        search.filter(&items, |s| s);
        assert_eq!(search.filtered_indices().len(), 0);

        search.pop_char();
        search.filter(&items, |s| s);
        assert_eq!(search.filtered_indices().len(), 1);
    }

    #[test]
    fn test_search_state_clear() {
        let items = vec!["feature-1"];
        let mut search = SearchState::new(items.len());
        search.push_char('x');
        search.clear();
        search.filter(&items, |s| s);

        assert_eq!(search.filtered_indices().len(), 1);
        assert!(search.is_empty());
    }

    #[test]
    fn test_highlight_matches_empty() {
        let spans = highlight_matches("feature-auth", &[], Style::default());
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn test_highlight_matches_with_indices() {
        let indices = vec![0, 8, 9, 10, 11]; // "f" and "auth"
        let spans = highlight_matches("feature-auth", &indices, Style::default());
        // Should have spans for: "f", "eature-", "a", "u", "t", "h"
        assert!(spans.len() > 1);
    }
}
