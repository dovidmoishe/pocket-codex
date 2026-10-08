# Verification record

Checked on 2026-10-08 on Windows with Rust 1.97.1 and Codex CLI 0.160.1.

- Release compilation succeeded.
- Rust formatting, Clippy with warnings denied, and the JavaScript syntax check passed.
- Eight Rust tests passed, covering persistence/restart recovery, process locking,
  concurrent edits, authentication/CSRF, private image delivery, cancellation,
  native protocol event buffering and thread resumption, and image path confinement.
- The browser flow was exercised: login, streamed planning chat, mock generation,
  gallery image delivery, and selecting an existing image as a reference.
- Desktop and 390 × 844 mobile layouts were visually inspected.
- A **live native Codex generation** completed using the local signed-in account.
  The server persisted one PNG (1,167,698 bytes), associated it with its reply,
  and served an authenticated download with HTTP 200 and `image/png` content type.
- The Rust web processes were approximately 7–8 MB working set during this local
  check. This is an observation, not a memory guarantee, and excludes Codex and
  any inference-provider/plugin processes.

The live check's credentials, chat, and generated image are in ignored local data,
not in the repository. Automated tests use fixtures and incur no inference usage.
Cross-platform CI is configured but its remote runs are not claimed as completed.
An actual public ngrok tunnel was not opened; an ngrok account/token must be
configured locally to use the documented tunnel commands.
