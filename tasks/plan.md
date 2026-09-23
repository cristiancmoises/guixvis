# Guixvis 0.6.0 implementation plan

The execution brief is [the improvement prompt](../docs/improvement-prompt.md).
The user has authorized implementation, documentation, commit, and pushes to
the existing remotes before 17:50 Brazil time on September 23, 2026.

## Independent slices

1. Core performance and security: inspect search allocations and cache writes;
   harden the web request boundary. Preserve API shapes and add regression tests.
2. Browser package view and themes: provide a persistent result list and safe
   package command copying; support system themes and fix asynchronous races.
3. Emacs: add asynchronous native search/details and reliable terminal reuse;
   validate the local service URL and provide actionable connection errors.
4. Terminal themes: add a CLI theme override and remembered choices while
   preserving `NO_COLOR` behavior.
5. Documentation and delivery: explain actual behavior in both READMEs, add
   usage/security/release notes, verify all slices, review, bump to 0.6.0,
   commit as the configured owner, and push to all four configured remotes.

The first four slices may proceed in parallel with separate file ownership.
Documentation follows implementation. Final review and publication are serial.
No dependency additions are planned. No web endpoint will mutate Guix state.

## Acceptance and checks

- Existing ranking and graph behavior pass Rust fixture tests; new security
  boundaries and edge cases receive focused regression coverage.
- Browser search, details, copied commands, keyboard navigation, and themes
  work at desktop and narrow widths without new console errors.
- Emacs source loads and compiles; mock HTTP/process tests cover native browsing
  and terminal reuse without installing or removing any actual packages.
- Cargo formatting, Clippy, default/all-feature tests and a release build pass.
- Documentation distinguishes fresh measurements from historical figures.
- All remote identities and final commit IDs are checked without printing tokens.

## Risks

The deadline limits breadth. Prefer complete, verified improvements over new
subsystems. Guix and forge availability may limit live checks; report those
limits explicitly. If the cutoff arrives, retain local work and do not publish.
