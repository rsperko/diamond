//! Stack visualization for PR descriptions
//!
//! This module generates markdown tables showing the stack of PRs
//! and updates PR descriptions with this information.

use anyhow::Result;
use colored::Colorize;
use regex::Regex;
use std::sync::LazyLock;

use crate::cache::Cache;
use crate::forge::{AsyncForge, Forge, PrFullInfo, PrState};
use crate::ref_store::RefStore;
use crate::ui::{PrProgressTracker, PrStatus};

/// Precompiled regex for versioned stack section start markers
static VERSIONED_START_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!-- dm:stack:v\d+:[a-f0-9]+:start -->").unwrap());

/// Precompiled regex for versioned stack section end markers
static VERSIONED_END_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!-- dm:stack:v\d+:[a-f0-9]+:end -->").unwrap());

/// Delimiter for the start of the Diamond stack section
pub const STACK_START: &str = "<!-- diamond:stack:start -->";
/// Delimiter for the end of the Diamond stack section
pub const STACK_END: &str = "<!-- diamond:stack:end -->";

/// Check if a branch name contains dangerous patterns
///
/// These patterns could enable link injection, code block manipulation,
/// or HTML comment abuse in PR descriptions.
///
/// Note: Git itself restricts some characters in branch names, but not all
/// of these patterns. This provides defense-in-depth for markdown injection.
pub fn is_dangerous_branch_name(name: &str) -> bool {
    // URL patterns that could create fake links (markdown link injection)
    name.contains("](http")
        || name.contains("](https")
        || name.contains("](javascript")
        || name.contains("](file")  // file:// protocol
        || name.contains("](data")  // data: URIs
        // Image injection (markdown images can have onclick handlers in some renderers)
        || name.contains("![")
        // Code block manipulation
        || name.contains("```")
        // HTML comment manipulation (could break out of comments)
        || name.contains("<!--")
        || name.contains("-->")
        // Raw HTML tags (some markdown renderers allow HTML)
        || name.contains("<script")
        || name.contains("<img")
        || name.contains("<iframe")
        || name.contains("<object")
        || name.contains("<embed")
        || name.contains("<svg")
        || name.contains("<a ")
        || name.contains("<a>")
        // Event handlers
        || name.to_lowercase().contains("onerror")
        || name.to_lowercase().contains("onload")
        || name.to_lowercase().contains("onclick")
        // Unicode control characters (RTL override, zero-width, etc.)
        || name.chars().any(|c| {
            matches!(c,
                '\u{200B}'..='\u{200F}' |  // Zero-width and directional
                '\u{202A}'..='\u{202E}' |  // Directional formatting
                '\u{2066}'..='\u{2069}' |  // Directional isolates
                '\u{FEFF}'                  // BOM / zero-width no-break space
            )
        })
}

/// Find the stack section in a PR description
///
/// Returns the start and end byte positions (inclusive of markers) if found.
/// Supports both versioned markers (`<!-- dm:stack:v1:hash:start -->`)
/// and legacy markers (`<!-- diamond:stack:start -->`).
pub fn find_stack_section(body: &str) -> Option<(usize, usize)> {
    // Try versioned markers first (using precompiled static regexes)
    if let Some(start_match) = VERSIONED_START_RE.find(body) {
        if let Some(end_match) = VERSIONED_END_RE.find(&body[start_match.end()..]) {
            let end_pos = start_match.end() + end_match.end();
            return Some((start_match.start(), end_pos));
        }
    }

    // Fall back to legacy markers
    let start_idx = body.find(STACK_START)?;
    let end_idx = body.find(STACK_END)?;
    if start_idx < end_idx {
        Some((start_idx, end_idx + STACK_END.len()))
    } else {
        None
    }
}

/// Non-breaking space character (prevents markdown from collapsing whitespace)
const NBSP: char = '\u{00A0}';

/// Get the status text for a PR
fn status_text(pr: &PrFullInfo) -> &'static str {
    if pr.is_draft {
        "Draft"
    } else {
        match pr.state {
            PrState::Open => "Open",
            PrState::Merged => "Merged",
            PrState::Closed => "Closed",
        }
    }
}

/// Generate the markdown table for a stack of PRs
///
/// Uses a collapsible `<details>` format with a table showing
/// tree structure, title, and status.
///
/// # Arguments
/// * `stack` - The PRs in the stack, in parent-first order
/// * `current_head_ref` - The head ref of the "current" PR (will be highlighted)
/// * `ref_store` - Optional RefStore for computing tree structure (None = flat display)
///
/// # Returns
/// The markdown string for the stack section (including delimiters)
pub fn generate_stack_markdown(stack: &[PrFullInfo], current_head_ref: &str, ref_store: Option<&RefStore>) -> String {
    if stack.is_empty() {
        return String::new();
    }

    // Find current PR position for the summary
    let current_pos = stack
        .iter()
        .position(|pr| pr.head_ref == current_head_ref)
        .map(|i| i + 1)
        .unwrap_or(1);
    let total = stack.len();

    // Root is the first branch in the stack
    let root = &stack[0].head_ref;

    let mut lines = vec![
        STACK_START.to_string(),
        format!(
            "<details>\n<summary>📚 Stack ({} of {}) · <a href=\"https://github.com/rsperko/diamond\">Diamond</a></summary>",
            current_pos, total
        ),
        String::new(),
        "| PR | Title | Status |".to_string(),
        "|:---|:---|:---:|".to_string(),
    ];

    for pr in stack {
        let is_current = pr.head_ref == current_head_ref;
        let is_inactive = pr.state == PrState::Merged || pr.state == PrState::Closed;

        // Compute tree prefix (box-drawing characters showing hierarchy)
        let tree_prefix = ref_store
            .map(|rs| rs.compute_tree_prefix(&pr.head_ref, root))
            .unwrap_or_default();

        // Current marker: ▶ for current PR only
        let current_marker = if is_current { "▶" } else { "" };

        let pr_link = format!("[#{}]({})", pr.number, pr.url);
        let title = truncate_title(&pr.title, 50);
        let status = status_text(pr);

        // Build the cells with appropriate formatting
        let (pr_cell, title_cell, status_cell) = if is_inactive {
            // Merged/closed: strikethrough
            (
                format!("{}{}{}~~{}~~", tree_prefix, current_marker, NBSP, pr_link),
                format!("~~{}~~", title),
                format!("~~{}~~", status),
            )
        } else if is_current {
            // Current: bold
            (
                format!("{}{}{}**{}**", tree_prefix, current_marker, NBSP, pr_link),
                format!("**{}**", title),
                format!("**{}**", status),
            )
        } else {
            // Normal
            (format!("{}{}", tree_prefix, pr_link), title, status.to_string())
        };

        lines.push(format!("| {} | {} | {} |", pr_cell, title_cell, status_cell));
    }

    lines.push(String::new());
    lines.push("</details>".to_string());
    lines.push(STACK_END.to_string());

    lines.join("\n")
}

/// Truncate a title to a maximum number of characters, adding ellipsis if needed
fn truncate_title(title: &str, max_chars: usize) -> String {
    let char_count = title.chars().count();
    if char_count <= max_chars {
        title.to_string()
    } else {
        // Take max_chars - 1 characters and add ellipsis
        let truncated: String = title.chars().take(max_chars - 1).collect();
        format!("{}…", truncated)
    }
}

/// Update a PR description with the stack visualization
///
/// This preserves any user content in the description and replaces
/// only the Diamond stack section (or appends it if not present).
///
/// # Arguments
/// * `original_body` - The current PR body/description
/// * `stack_markdown` - The new stack markdown to insert
///
/// # Returns
/// The updated PR body with the stack section
pub fn update_pr_description(original_body: &str, stack_markdown: &str) -> String {
    let user_content = extract_user_content(original_body);

    if stack_markdown.is_empty() {
        return user_content;
    }

    if user_content.is_empty() {
        stack_markdown.to_string()
    } else {
        format!("{}\n\n{}", user_content.trim_end(), stack_markdown)
    }
}

/// Extract user content from a PR body, removing the Diamond stack section
///
/// # Arguments
/// * `body` - The PR body/description
///
/// # Returns
/// The user content without the Diamond stack section
pub fn extract_user_content(body: &str) -> String {
    match find_stack_section(body) {
        Some((start, end)) => {
            // Found valid delimiters - remove the section
            let before = &body[..start];
            let after = &body[end..];

            // Trim extra whitespace between sections
            let before = before.trim_end();
            let after = after.trim_start();

            if before.is_empty() {
                after.to_string()
            } else if after.is_empty() {
                before.to_string()
            } else {
                format!("{}\n\n{}", before, after)
            }
        }
        None => {
            // No valid delimiters found - return original
            body.to_string()
        }
    }
}

/// Collect all branches in the same stack (ancestors and descendants)
///
/// Starting from any branch in the stack, this walks up to find the root
/// (first branch after trunk), then collects all descendants via DFS.
pub fn collect_full_stack(branch: &str, ref_store: &RefStore) -> Result<Vec<String>> {
    let trunk = ref_store.get_trunk()?.unwrap_or_default();

    // Walk up to find the root (first branch after trunk) with cycle detection
    let mut root = branch.to_string();
    let mut seen = std::collections::HashSet::new();
    seen.insert(root.clone());

    while let Some(parent) = ref_store.get_parent(&root)? {
        if parent == trunk {
            break; // Reached trunk, root is current
        }

        // Cycle detection
        if !seen.insert(parent.clone()) {
            anyhow::bail!(
                "Circular parent reference detected while walking stack. \
                 Run 'dm cleanup' to repair metadata."
            );
        }

        if ref_store.is_tracked(&parent)? {
            root = parent;
        } else {
            break;
        }
    }

    // Collect all branches in DFS order from root
    ref_store.collect_branches_dfs(&[root])
}

/// Update the stack visualization in all open PRs
///
/// This fetches full info for all PRs, generates the stack markdown,
/// and updates each open PR's description. Merged/closed PRs are shown
/// in the visualization with strikethrough but their descriptions are
/// not updated.
///
/// # Arguments
/// * `branches` - The branches in the stack
/// * `forge` - The forge to use for PR operations
/// * `ref_store` - The ref store for computing tree depth
/// * `verbose` - Whether to print progress
///
/// # Returns
/// The number of PRs updated
pub fn update_stack_visualization(
    branches: &[String],
    forge: &dyn Forge,
    ref_store: &RefStore,
    verbose: bool,
) -> Result<usize> {
    // Collect full info for all branches that have PRs
    let mut pr_infos: Vec<PrFullInfo> = Vec::new();

    for branch in branches {
        match forge.pr_exists(branch)? {
            Some(_) => {
                // Get full info for this PR
                match forge.get_pr_full_info(branch) {
                    Ok(info) => pr_infos.push(info),
                    Err(e) => {
                        eprintln!("{} Could not get full PR info for {}: {}", "⚠".yellow(), branch, e);
                    }
                }
            }
            None => {
                // Branch doesn't have a PR yet - skip
            }
        }
    }

    if pr_infos.is_empty() {
        return Ok(0);
    }

    // Update each PR's description (skip merged/closed - they can't be edited)
    let mut updated_count = 0;
    for pr in &pr_infos {
        // Skip merged and closed PRs - their descriptions can't be updated
        // but they're still shown in the stack visualization with strikethrough
        if pr.state == PrState::Merged || pr.state == PrState::Closed {
            continue;
        }

        if verbose {
            print!("  Updating PR #{}...", pr.number);
            use std::io::Write;
            std::io::stdout().flush().ok();
        }

        // Generate stack markdown for this PR (includes all PRs with strikethrough for merged/closed)
        let stack_md = generate_stack_markdown(&pr_infos, &pr.head_ref, Some(ref_store));

        // Get current body and update with stack
        match forge.get_pr_body(&pr.number.to_string()) {
            Ok(current_body) => {
                let new_body = update_pr_description(&current_body, &stack_md);
                if let Err(e) = forge.update_pr_body(&pr.number.to_string(), &new_body) {
                    if verbose {
                        println!(" {}", "failed".red());
                    }
                    eprintln!("{} Failed to update PR #{}: {}", "⚠".yellow(), pr.number, e);
                } else {
                    if verbose {
                        println!(" {}", "✓".green());
                    }
                    updated_count += 1;
                }
            }
            Err(e) => {
                if verbose {
                    println!(" {}", "failed".red());
                }
                eprintln!("{} Could not get body for PR #{}: {}", "⚠".yellow(), pr.number, e);
            }
        }
    }

    Ok(updated_count)
}

/// Update the stack visualization in all open PRs (async version)
///
/// This version uses batch operations for parallel API calls, providing
/// significant performance improvements for stacks with many PRs.
///
/// Shows beautiful progress with individual PR status in TTY mode.
///
/// # Arguments
/// * `branches` - The branches in the stack
/// * `forge` - The async forge to use for PR operations
/// * `ref_store` - The ref store for computing tree depth
/// * `verbose` - Whether to print progress
///
/// # Returns
/// The number of PRs updated
pub async fn update_stack_visualization_async(
    branches: &[String],
    forge: &dyn AsyncForge,
    ref_store: &RefStore,
    verbose: bool,
) -> Result<usize> {
    if branches.is_empty() {
        return Ok(0);
    }

    // Create progress tracker if verbose
    let tracker = if verbose {
        Some(PrProgressTracker::new("Updating stack visualization..."))
    } else {
        None
    };

    // Batch fetch all PR info in parallel
    let pr_infos = forge.get_prs_full_info(branches).await;

    if pr_infos.is_empty() {
        if let Some(t) = &tracker {
            t.finish(0, 0, 0);
        }
        return Ok(0);
    }

    // Add all PRs to tracker and mark as fetching
    if let Some(t) = &tracker {
        for pr in &pr_infos {
            t.add_pr(pr.number, &pr.head_ref);
            t.update_status(pr.number, PrStatus::Fetching);
        }
    }

    // Filter to only open PRs that can be updated
    let open_prs: Vec<&PrFullInfo> = pr_infos
        .iter()
        .filter(|pr| pr.state != PrState::Merged && pr.state != PrState::Closed)
        .collect();

    // Mark closed/merged PRs as skipped
    if let Some(t) = &tracker {
        for pr in &pr_infos {
            if pr.state == PrState::Merged || pr.state == PrState::Closed {
                t.update_status(pr.number, PrStatus::Skipped);
            }
        }
    }

    if open_prs.is_empty() {
        if let Some(t) = &tracker {
            t.finish(0, pr_infos.len(), 0);
        }
        return Ok(0);
    }

    // Batch fetch all PR bodies in parallel
    let pr_refs: Vec<String> = open_prs.iter().map(|pr| pr.number.to_string()).collect();
    let bodies = forge.get_pr_bodies(&pr_refs).await;

    // Build a map of pr_number -> current_body
    let body_map: std::collections::HashMap<String, String> = bodies.into_iter().collect();

    // Generate new bodies for each PR
    let mut updates: Vec<(String, String)> = Vec::new();
    let mut failed_count = 0;

    for pr in &open_prs {
        let pr_ref = pr.number.to_string();

        if let Some(t) = &tracker {
            t.update_status(pr.number, PrStatus::Generating);
        }

        if let Some(current_body) = body_map.get(&pr_ref) {
            // Generate stack markdown for this PR
            let stack_md = generate_stack_markdown(&pr_infos, &pr.head_ref, Some(ref_store));
            let new_body = update_pr_description(current_body, &stack_md);
            updates.push((pr_ref, new_body));

            if let Some(t) = &tracker {
                t.update_status(pr.number, PrStatus::Updating);
            }
        } else {
            if let Some(t) = &tracker {
                t.update_status(pr.number, PrStatus::Failed);
            }
            failed_count += 1;
        }
    }

    // Batch update all PR bodies in parallel
    let updated_count = forge.update_pr_bodies(&updates).await;

    // Mark successful updates as done
    if let Some(t) = &tracker {
        for (pr_ref, _) in &updates {
            if let Ok(number) = pr_ref.parse::<u64>() {
                t.update_status(number, PrStatus::Done);
            }
        }
    }

    let skipped_count = pr_infos.len() - open_prs.len();

    if let Some(t) = &tracker {
        t.finish(updated_count, skipped_count, failed_count);
    }

    Ok(updated_count)
}

/// Update stack visualization for all tracked branches that have PRs
///
/// This is useful for repairing stack visualization in existing PRs.
///
/// # Arguments
/// * `ref_store` - The ref store for branch relationships
/// * `cache` - The cache for PR URLs
/// * `forge` - The forge to use for PR operations
/// * `verbose` - Whether to print progress
///
/// # Returns
/// The number of PRs updated
pub fn update_all_stack_visualizations(
    ref_store: &RefStore,
    cache: &Cache,
    forge: &dyn Forge,
    verbose: bool,
) -> Result<usize> {
    let trunk = ref_store.get_trunk()?.unwrap_or_default();
    let all_branches = ref_store.list_tracked_branches()?;

    // Find all distinct stacks (groups of connected branches)
    let mut processed: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut total_updated = 0;

    for branch in &all_branches {
        if processed.contains(branch) || branch == &trunk {
            continue;
        }

        // Get the full stack for this branch
        let stack = collect_full_stack(branch, ref_store)?;

        // Mark all branches in this stack as processed
        for b in &stack {
            processed.insert(b.clone());
        }

        // Update visualization for this stack
        let updated = update_stack_visualization(&stack, forge, ref_store, verbose)?;
        total_updated += updated;
    }

    // Suppress unused variable warning - cache will be used for future features
    let _ = cache;

    Ok(total_updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::{CiStatus, ReviewState};

    fn make_test_pr(number: u64, head_ref: &str, is_draft: bool) -> PrFullInfo {
        PrFullInfo {
            number,
            url: format!("https://github.com/user/repo/pull/{}", number),
            title: format!("PR {}", number),
            state: PrState::Open,
            is_draft,
            review: ReviewState::Pending,
            ci: CiStatus::Success,
            head_ref: head_ref.to_string(),
            base_ref: "main".to_string(),
        }
    }

    #[test]
    fn test_find_stack_section_with_versioned_markers() {
        // Backwards compatibility: should still find versioned markers
        let body = "User content\n\n<!-- dm:stack:v1:a1b2c3d4:start -->\nStack content\n<!-- dm:stack:v1:a1b2c3d4:end -->\n\nMore content";

        let result = find_stack_section(body);
        assert!(result.is_some());

        let (start, end) = result.unwrap();
        assert!(start < end);
        assert!(&body[start..].starts_with("<!-- dm:stack:v1:"));
        assert!(body[..end].ends_with(":end -->"));
    }

    #[test]
    fn test_find_stack_section_no_markers() {
        let body = "Just some user content without markers";

        let result = find_stack_section(body);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_stack_section_with_legacy_markers() {
        let body =
            "User content\n\n<!-- diamond:stack:start -->\nStack content\n<!-- diamond:stack:end -->\n\nMore content";

        let result = find_stack_section(body);
        assert!(result.is_some(), "Should find legacy markers");
    }

    #[test]
    fn test_is_dangerous_branch_name_safe() {
        assert!(!is_dangerous_branch_name("feature-auth"));
        assert!(!is_dangerous_branch_name("fix_bug_123"));
        assert!(!is_dangerous_branch_name("release-v2.0.0"));
    }

    #[test]
    fn test_is_dangerous_branch_name_url_injection() {
        // These patterns could create fake links in markdown
        assert!(is_dangerous_branch_name("branch](http://evil.com)"));
        assert!(is_dangerous_branch_name("branch](https://evil.com)"));
        assert!(is_dangerous_branch_name("branch](javascript:alert(1))"));
    }

    #[test]
    fn test_is_dangerous_branch_name_code_block() {
        // Could close/reopen code blocks
        assert!(is_dangerous_branch_name("branch```code"));
    }

    #[test]
    fn test_is_dangerous_branch_name_html_comment() {
        // Could manipulate HTML comments
        assert!(is_dangerous_branch_name("branch<!--"));
        assert!(is_dangerous_branch_name("branch-->"));
    }

    #[test]
    fn test_is_dangerous_branch_name_file_protocol() {
        // file:// and data: URIs
        assert!(is_dangerous_branch_name("branch](file://etc/passwd)"));
        assert!(is_dangerous_branch_name("branch](data:text/html,<script>)"));
    }

    #[test]
    fn test_is_dangerous_branch_name_image_injection() {
        // Markdown image syntax
        assert!(is_dangerous_branch_name("branch![alt](http://evil.com/img.png)"));
    }

    #[test]
    fn test_is_dangerous_branch_name_html_tags() {
        // Raw HTML tags
        assert!(is_dangerous_branch_name("branch<script>alert(1)</script>"));
        assert!(is_dangerous_branch_name("branch<img src=x>"));
        assert!(is_dangerous_branch_name("branch<iframe src=x>"));
        assert!(is_dangerous_branch_name("branch<svg onload=x>"));
        assert!(is_dangerous_branch_name("branch<a href=x>click</a>"));
        assert!(is_dangerous_branch_name("branch<a >link"));
    }

    #[test]
    fn test_is_dangerous_branch_name_event_handlers() {
        // Event handlers (case insensitive)
        assert!(is_dangerous_branch_name("branch-onerror=alert(1)"));
        assert!(is_dangerous_branch_name("branch-ONCLICK=x"));
        assert!(is_dangerous_branch_name("branch-onLoad"));
    }

    #[test]
    fn test_is_dangerous_branch_name_unicode_control() {
        // Unicode control characters
        assert!(is_dangerous_branch_name("branch\u{200B}hidden")); // Zero-width space
        assert!(is_dangerous_branch_name("branch\u{202E}reversed")); // RTL override
        assert!(is_dangerous_branch_name("branch\u{FEFF}bom")); // BOM
    }

    #[test]
    fn test_generate_stack_markdown_empty() {
        let result = generate_stack_markdown(&[], "feature", None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_generate_stack_markdown_single_pr() {
        let stack = vec![make_test_pr(123, "feature", false)];
        let result = generate_stack_markdown(&stack, "feature", None);

        assert!(result.contains(STACK_START));
        assert!(result.contains(STACK_END));
        assert!(result.contains("<details>"));
        assert!(result.contains("</details>"));
        assert!(result.contains("Stack (1 of 1)"));
        assert!(result.contains("Diamond"));
        assert!(result.contains("**[#123]")); // Current PR is bold
        assert!(result.contains("▶")); // Current marker (▶ instead of status emoji)
    }

    #[test]
    fn test_generate_stack_markdown_multiple_prs() {
        let stack = vec![
            make_test_pr(101, "feature-1", false),
            make_test_pr(102, "feature-2", false),
            make_test_pr(103, "feature-3", true), // Draft PR
        ];
        let result = generate_stack_markdown(&stack, "feature-2", None);

        // Should show position
        assert!(result.contains("Stack (2 of 3)"));

        // PR 101 should have Open status (not current)
        assert!(result.contains("[#101]"));
        assert!(result.contains("| Open |")); // Status text for non-current open PR

        // PR 102 should be marked as current with bold and ▶ marker
        assert!(result.contains("**[#102]"));
        assert!(result.contains("▶")); // Current marker

        // PR 103 (draft) should have Draft status
        assert!(result.contains("[#103]"));
        assert!(result.contains("| Draft |")); // Draft status text
    }

    #[test]
    fn test_generate_stack_markdown_with_merged_pr() {
        let mut pr1 = make_test_pr(101, "feature-1", false);
        pr1.state = PrState::Merged;

        let pr2 = make_test_pr(102, "feature-2", false);
        let stack = vec![pr1, pr2];

        let result = generate_stack_markdown(&stack, "feature-2", None);

        // Both PRs should be in the table
        assert!(result.contains("[#101]"));
        assert!(result.contains("[#102]"));
        assert!(result.contains("Stack (2 of 2)"));
        // Merged PR should have strikethrough and Merged status
        assert!(result.contains("~~"));
        assert!(result.contains("~~Merged~~")); // Merged status with strikethrough
    }

    #[test]
    fn test_truncate_title_short() {
        assert_eq!(truncate_title("Short title", 40), "Short title");
    }

    #[test]
    fn test_truncate_title_long() {
        let long_title = "This is a very long title that exceeds the maximum length allowed";
        let result = truncate_title(long_title, 40);
        // Check character count, not byte length (ellipsis is multi-byte)
        assert!(result.chars().count() <= 40);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn test_extract_user_content_no_stack() {
        let body = "This is my PR description.\n\nIt has multiple paragraphs.";
        assert_eq!(extract_user_content(body), body);
    }

    #[test]
    fn test_extract_user_content_with_stack() {
        let body = format!(
            "This is my PR description.\n\n{}\n## Stack\n...\n{}\n\nMore content",
            STACK_START, STACK_END
        );
        let result = extract_user_content(&body);
        assert!(result.contains("This is my PR description."));
        assert!(result.contains("More content"));
        assert!(!result.contains(STACK_START));
        assert!(!result.contains(STACK_END));
    }

    #[test]
    fn test_extract_user_content_stack_at_end() {
        let body = format!(
            "This is my PR description.\n\n{}\n## Stack\n...\n{}",
            STACK_START, STACK_END
        );
        let result = extract_user_content(&body);
        assert_eq!(result.trim(), "This is my PR description.");
    }

    #[test]
    fn test_extract_user_content_stack_only() {
        let body = format!("{}\n## Stack\n...\n{}", STACK_START, STACK_END);
        let result = extract_user_content(&body);
        assert!(result.is_empty());
    }

    #[test]
    fn test_update_pr_description_empty_body() {
        let stack_md = generate_stack_markdown(&[make_test_pr(123, "feature", false)], "feature", None);
        let result = update_pr_description("", &stack_md);
        assert!(result.contains(STACK_START));
    }

    #[test]
    fn test_update_pr_description_with_existing_content() {
        let original = "My PR description.";
        let stack_md = generate_stack_markdown(&[make_test_pr(123, "feature", false)], "feature", None);
        let result = update_pr_description(original, &stack_md);

        assert!(result.starts_with("My PR description."));
        assert!(result.contains(STACK_START));
    }

    #[test]
    fn test_update_pr_description_replaces_existing_stack() {
        let original = format!(
            "My PR description.\n\n{}\nOld stack content\n{}",
            STACK_START, STACK_END
        );
        let stack_md = generate_stack_markdown(&[make_test_pr(123, "feature", false)], "feature", None);
        let result = update_pr_description(&original, &stack_md);

        assert!(result.contains("My PR description."));
        assert!(!result.contains("Old stack content"));
        assert!(result.contains("[#123]"));
        // Should only have one stack section
        assert_eq!(result.matches(STACK_START).count(), 1);
    }

    #[test]
    fn test_update_pr_description_empty_stack() {
        let original = "My PR description.";
        let result = update_pr_description(original, "");
        assert_eq!(result, "My PR description.");
    }

    // =========================================================================
    // Status Text Tests
    // =========================================================================

    #[test]
    fn test_status_text_open() {
        let pr = make_test_pr(1, "feature", false);
        assert_eq!(status_text(&pr), "Open");
    }

    #[test]
    fn test_status_text_draft() {
        let pr = make_test_pr(1, "feature", true);
        assert_eq!(status_text(&pr), "Draft");
    }

    #[test]
    fn test_status_text_merged() {
        let mut pr = make_test_pr(1, "feature", false);
        pr.state = PrState::Merged;
        assert_eq!(status_text(&pr), "Merged");
    }

    #[test]
    fn test_status_text_closed() {
        let mut pr = make_test_pr(1, "feature", false);
        pr.state = PrState::Closed;
        assert_eq!(status_text(&pr), "Closed");
    }

    #[test]
    fn test_status_text_draft_takes_precedence() {
        // Even if PR state is Merged, draft flag should show Draft
        let mut pr = make_test_pr(1, "feature", true);
        pr.state = PrState::Merged;
        assert_eq!(status_text(&pr), "Draft");
    }

    // =========================================================================
    // Status Column Integration Tests
    // =========================================================================

    #[test]
    fn test_markdown_contains_status_column_header() {
        let stack = vec![make_test_pr(101, "feature-1", false)];
        let result = generate_stack_markdown(&stack, "feature-1", None);

        assert!(result.contains("| PR | Title | Status |"));
    }

    #[test]
    fn test_markdown_contains_current_marker() {
        let stack = vec![make_test_pr(101, "feature-1", false)];
        let result = generate_stack_markdown(&stack, "feature-1", None);

        // Current PR should have ▶ marker
        assert!(result.contains("▶"));
    }

    #[test]
    fn test_markdown_contains_merged_status_text() {
        let mut pr1 = make_test_pr(101, "feature-1", false);
        pr1.state = PrState::Merged;
        let pr2 = make_test_pr(102, "feature-2", false);
        let stack = vec![pr1, pr2];

        let result = generate_stack_markdown(&stack, "feature-2", None);

        // Merged PR should have strikethrough status
        assert!(result.contains("~~Merged~~"));
    }

    #[test]
    fn test_markdown_contains_draft_status_text() {
        let pr1 = make_test_pr(101, "feature-1", true); // draft
        let pr2 = make_test_pr(102, "feature-2", false);
        let stack = vec![pr1, pr2];

        let result = generate_stack_markdown(&stack, "feature-2", None);

        // Draft PR should have Draft in status column
        assert!(result.contains("| Draft |"));
    }

    #[test]
    fn test_markdown_contains_closed_status_text() {
        let mut pr1 = make_test_pr(101, "feature-1", false);
        pr1.state = PrState::Closed;
        let pr2 = make_test_pr(102, "feature-2", false);
        let stack = vec![pr1, pr2];

        let result = generate_stack_markdown(&stack, "feature-2", None);

        // Closed PR should have strikethrough status
        assert!(result.contains("~~Closed~~"));
    }

    // =========================================================================
    // Strikethrough and Edge Case Tests
    // =========================================================================

    #[test]
    fn test_generate_stack_markdown_with_closed_pr() {
        let mut pr1 = make_test_pr(101, "feature-1", false);
        pr1.state = PrState::Closed;

        let pr2 = make_test_pr(102, "feature-2", false);
        let stack = vec![pr1, pr2];

        let result = generate_stack_markdown(&stack, "feature-2", None);

        // Closed PR should have strikethrough and Closed status
        assert!(result.contains("~~"));
        assert!(result.contains("~~Closed~~")); // Closed status with strikethrough
        assert!(result.contains("[#101]"));
    }

    #[test]
    fn test_generate_stack_markdown_current_pr_merged() {
        // Edge case: viewing a merged PR's description
        let mut pr1 = make_test_pr(101, "feature-1", false);
        pr1.state = PrState::Merged;

        let stack = vec![pr1];

        // Current PR is the merged one
        let result = generate_stack_markdown(&stack, "feature-1", None);

        // Current PR uses ▶ marker, with strikethrough for merged
        assert!(result.contains("▶")); // Current marker
        assert!(result.contains("~~")); // Strikethrough for merged
        assert!(result.contains("~~Merged~~")); // Merged status with strikethrough
    }

    #[test]
    fn test_generate_stack_markdown_current_pr_closed() {
        // Edge case: viewing a closed PR's description
        let mut pr1 = make_test_pr(101, "feature-1", false);
        pr1.state = PrState::Closed;

        let stack = vec![pr1];

        // Current PR is the closed one
        let result = generate_stack_markdown(&stack, "feature-1", None);

        // Current PR uses ▶ marker, with strikethrough for closed
        assert!(result.contains("▶")); // Current marker
        assert!(result.contains("~~")); // Strikethrough for closed
        assert!(result.contains("~~Closed~~")); // Closed status with strikethrough
    }

    #[test]
    fn test_generate_stack_markdown_mixed_states() {
        // Stack with various PR states
        let mut merged = make_test_pr(101, "feature-1", false);
        merged.state = PrState::Merged;

        let open = make_test_pr(102, "feature-2", false);

        let draft = make_test_pr(103, "feature-3", true);

        let mut closed = make_test_pr(104, "feature-4", false);
        closed.state = PrState::Closed;

        let stack = vec![merged, open, draft, closed];
        let result = generate_stack_markdown(&stack, "feature-2", None);

        // Verify non-current PRs have their status text
        assert!(result.contains("~~Merged~~")); // Merged (#101) with strikethrough
        assert!(result.contains("| Draft |")); // Draft (#103)
        assert!(result.contains("~~Closed~~")); // Closed (#104) with strikethrough

        // Current (feature-2) should be bold with ▶ marker
        assert!(result.contains("**[#102]"));
        assert!(result.contains("▶")); // Current marker

        // Check stack position
        assert!(result.contains("Stack (2 of 4)"));
    }
}
