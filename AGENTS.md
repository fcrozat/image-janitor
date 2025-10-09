# Rules for AGENTS

## Dev environment tips
- When working on a feature, first implement the changes, check it builds and passes existing tests, create a git commit, then create a testcase and do a separate git commit as fixup
- Never try to do git push

## Testing instructions
- From the package root you can just call `cargo test`. The commit should pass all tests before you commit anything
- Fix any test or type errors until the whole suite is green.
- Add or update tests for the code you change, even if nobody asked.

## Commit instruction
- Always run `cargo clippy` and `cargo test` before committing.
