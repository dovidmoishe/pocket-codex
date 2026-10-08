# Contributing guidance

Keep this app small: Rust, static browser assets, JSON metadata, and files on disk.
Do not introduce a database or a frontend build system without discussing it first.
Never commit credentials, private chats, generated images, or Codex runtime data.
Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.

Use Conventional Commits: `type(scope): description`. Use lowercase, imperative
descriptions with no trailing period. Scopes: `web`, `api`, `repo`, `deps`, `ci`.
