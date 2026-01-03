//! Execution context for Diamond CLI.
//!
//! Provides task-local storage for global flags like --verbose and --dry-run.
//! Uses tokio::task_local to ensure context is preserved across async task migrations.
//! This avoids passing flags through every function signature while remaining async-safe.

use std::cell::RefCell;
use std::future::Future;

// Thread-local fallback for synchronous code paths
thread_local! {
    static SYNC_CONTEXT: RefCell<ExecutionContext> = RefCell::new(ExecutionContext::default());
}

// Task-local for async code paths (preserved across .await points and thread migrations)
tokio::task_local! {
    static ASYNC_CONTEXT: ExecutionContext;
}

/// Global execution context for the current CLI invocation
#[derive(Clone, Copy, Default)]
pub struct ExecutionContext {
    /// Show git commands being executed
    pub verbose: bool,
    /// Preview operations without executing them
    pub dry_run: bool,
}

impl ExecutionContext {
    /// Create a new execution context
    pub fn new(verbose: bool, dry_run: bool) -> Self {
        Self { verbose, dry_run }
    }

    /// Initialize the thread-local context (for synchronous code paths)
    ///
    /// This is a fallback for code that runs outside of `with_context`.
    /// For async code, prefer using `with_context` to properly scope the context.
    pub fn init(verbose: bool, dry_run: bool) {
        SYNC_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = ExecutionContext { verbose, dry_run };
        });
    }

    /// Check if verbose mode is enabled
    ///
    /// Checks task-local context first (for async code), falls back to thread-local.
    pub fn is_verbose() -> bool {
        // Try task-local first (async context)
        if let Ok(verbose) = ASYNC_CONTEXT.try_with(|ctx| ctx.verbose) {
            return verbose;
        }
        // Fall back to thread-local (sync context)
        SYNC_CONTEXT.with(|ctx| ctx.borrow().verbose)
    }

    /// Check if dry-run mode is enabled
    ///
    /// Checks task-local context first (for async code), falls back to thread-local.
    pub fn is_dry_run() -> bool {
        // Try task-local first (async context)
        if let Ok(dry_run) = ASYNC_CONTEXT.try_with(|ctx| ctx.dry_run) {
            return dry_run;
        }
        // Fall back to thread-local (sync context)
        SYNC_CONTEXT.with(|ctx| ctx.borrow().dry_run)
    }
}

/// Run an async function with the given execution context.
///
/// The context is properly propagated across .await points and thread migrations.
/// This is the preferred way to establish context for async code paths.
///
/// # Example
/// ```ignore
/// let ctx = ExecutionContext::new(verbose, dry_run);
/// with_context(ctx, async {
///     // ExecutionContext::is_verbose() works correctly here,
///     // even after .await points
///     some_async_operation().await;
///     println!("Verbose: {}", ExecutionContext::is_verbose());
/// }).await;
/// ```
pub async fn with_context<F, T>(ctx: ExecutionContext, f: F) -> T
where
    F: Future<Output = T>,
{
    ASYNC_CONTEXT.scope(ctx, f).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_context() {
        // Reset to default
        ExecutionContext::init(false, false);
        assert!(!ExecutionContext::is_verbose());
        assert!(!ExecutionContext::is_dry_run());
    }

    #[test]
    fn test_verbose_flag() {
        ExecutionContext::init(true, false);
        assert!(ExecutionContext::is_verbose());
        assert!(!ExecutionContext::is_dry_run());
    }

    #[test]
    fn test_dry_run_flag() {
        ExecutionContext::init(false, true);
        assert!(!ExecutionContext::is_verbose());
        assert!(ExecutionContext::is_dry_run());
    }

    #[test]
    fn test_both_flags() {
        ExecutionContext::init(true, true);
        assert!(ExecutionContext::is_verbose());
        assert!(ExecutionContext::is_dry_run());
    }

    #[tokio::test]
    async fn test_async_context_propagation() {
        let ctx = ExecutionContext::new(true, true);
        with_context(ctx, async {
            assert!(ExecutionContext::is_verbose());
            assert!(ExecutionContext::is_dry_run());

            // Simulate an await point
            tokio::task::yield_now().await;

            // Context should still be available after await
            assert!(ExecutionContext::is_verbose());
            assert!(ExecutionContext::is_dry_run());
        })
        .await;
    }

    #[tokio::test]
    async fn test_async_context_isolation() {
        // Set thread-local to false
        ExecutionContext::init(false, false);

        // Run with async context set to true
        let ctx = ExecutionContext::new(true, true);
        with_context(ctx, async {
            assert!(ExecutionContext::is_verbose());
            assert!(ExecutionContext::is_dry_run());
        })
        .await;

        // Thread-local should still be false
        assert!(!ExecutionContext::is_verbose());
        assert!(!ExecutionContext::is_dry_run());
    }
}
