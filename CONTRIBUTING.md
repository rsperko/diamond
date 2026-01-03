# Contributing to Diamond

First off, thanks for taking the time to contribute! ❤️

All types of contributions are encouraged and valued. See the [Table of Contents](#table-of-contents) for different ways to help and details about how this project handles them. Please make sure to read the relevant section before making your contribution. It will make it a lot easier for us maintainers and smooth out the experience for all involved. The community looks forward to your contributions.

## Table of Contents

- [I Have a Question](#i-have-a-question)
- [I Want To Contribute](#i-want-to-contribute)
  - [Reporting Bugs](#reporting-bugs)
  - [Suggesting Enhancements](#suggesting-enhancements)
  - [Your First Code Contribution](#your-first-code-contribution)

## I Have a Question

If you want to ask a question, we assume you have read the available [Documentation](docs/).

Before you ask a question, it is best to search for existing [Issues](https://github.com/rsperko/diamond/issues) that might help you. In case you have found a suitable issue and still need clarification, you can write your question in this issue. It is also advisable to search the internet for answers first.

## I Want To Contribute

### Reporting Bugs

**If you find a security vulnerability, please do NOT open an issue. Report it privately via GitHub Security Advisories: https://github.com/rsperko/diamond/security/advisories/new**

Before submitting bug reports, check that your issue has not already been reported.

### Suggesting Enhancements

This section guides you through submitting an enhancement suggestion for Diamond, **including completely new features and minor improvements to existing functionality**. Following these guidelines will help maintainers and the community to understand your suggestion and find related suggestions.

### Your First Code Contribution

1.  **Fork the repository** on GitHub.
2.  **Clone your fork** locally.
3.  **Setup the environment**:
    ```bash
    make setup-hooks
    ```
4.  **Create a branch** for your feature.
5.  **Make changes**.
6.  **Test your changes**:
    ```bash
    make check
    ```
7.  **Commit and push** your changes.
    - Note: Commits are protected by Gitleaks. Ensure no secrets are included.
8.  **Create a Pull Request**.

## Styleguides

### Commit Messages

-   Use the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) format.
-   Start with a verb (e.g., "Add", "Fix", "Update").

### Rust Code Style

-   We follow standard Rustfmt style.
-   Run `cargo fmt` before committing.
-   Ensure `cargo clippy` passes without warnings.
