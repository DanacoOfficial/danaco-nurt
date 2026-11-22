<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
## Description

Describe the change and its motivation. Link the related issue, for example
`Closes #123`.

## Changes

-

## Testing

Describe how the change was tested, including the operating systems.

## Checklist

- [ ] Commit messages follow Conventional Commits with a crate or area scope.
- [ ] Every commit is signed off (`git commit --signoff`).
- [ ] New source files start with the producer header and carry no per-file license identifier.
- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass.
- [ ] Tests are added or updated; `cargo test --workspace` passes.
- [ ] Public items are documented; `cargo doc --workspace --no-deps` passes without warnings.
- [ ] `cargo deny check` passes; new dependencies are justified above.
- [ ] User-visible changes are recorded in `CHANGELOG.md` under `Unreleased`.
- [ ] Compatibility of the language, the syntax tree, the report format and the C interface is preserved, or the breaking change is described above.
