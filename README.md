# Pocket Codex

A small, self-hosted creative desk for your local Codex CLI. Someone you trust can
plan graphics, upload references, generate images, and download them from a browser.
Your laptop runs the server; ngrok supplies the HTTPS link.

**Rust server. Static frontend. JSON chats. Images on disk. No app database,
frontend build pipeline, or Node server.**

## What it does

- Mobile-friendly planning chat and a Generate mode.
- Native Codex image generation and reference-based edits, when your CLI provider supports them.
- Project history, image gallery, downloads, and JSON conversation export.
- Streaming replies, a single worker, eight waiting slots, and a Stop button.
- Browser reconnection without cancelling the job.
- Atomic JSON replacement and recovery of interrupted jobs on server restart.
- Shared access-key login; authenticated images and downloads.
- Mock mode for UI development without inference or usage charges.

## Requirements

- Rust 1.88 or later to build.
- A recent Codex CLI, signed in locally. This implementation was checked against
  the generated protocol of Codex CLI 0.160.1. The protocol evolves; see
  [the app-server documentation](https://learn.chatgpt.com/docs/app-server).
- ngrok for remote access. Install and authenticate it using
  [ngrok's official instructions](https://ngrok.com/docs/start).

After building, you need only the executable, Codex CLI, and your data directory.
The browser files are embedded in the executable.

## Try it locally first

From this repository:

```sh
cargo build --release --locked
```

In PowerShell on Windows:

```powershell
.\target\release\pocket-codex.exe --mock
```

In a second PowerShell window, from the same repository:

```powershell
.\target\release\pocket-codex.exe --show-key
```

Open `http://127.0.0.1:8787` and paste that key into the login screen. Mock mode
streams a canned planning response and returns a clearly labelled image fixture.
**The fixture is not AI-generated artwork.**

Stop the server with Ctrl+C. For real Codex, omit `--mock`:

```powershell
codex login
.\target\release\pocket-codex.exe
```

If the desktop app's CLI isn't on PATH, the server detects the standard Windows
desktop installation. You can also set `--codex-bin` explicitly.

On macOS/Linux, replace `.\target\release\pocket-codex.exe` with
`./target/release/pocket-codex`.

## Share it through ngrok

1. Authenticate ngrok locally. Do not put its account token in this repository.
2. Start the tunnel in a terminal:

   ```sh
   ngrok http 8787
   ```

3. Copy its HTTPS origin, such as `https://your-domain.ngrok-free.app`.
4. Start the app with that exact origin:

   ```powershell
   .\target\release\pocket-codex.exe --public-origin https://your-domain.ngrok-free.app
   ```

5. Send your trusted guest the HTTPS link and access key privately. Use that URL
   on your own laptop too when `--public-origin` is set.

The flag enables Secure session cookies and locks mutating requests to the
configured origin. **Without it, only the local browser's mutation requests are
accepted.** If your ngrok URL changes, restart with the new origin and sign in again.
Some ngrok plans show a browser interstitial; continue through it to reach the app.

The tunnel forwards to the Rust service bound to `127.0.0.1`. Codex communicates
with Rust over private process pipes; its control protocol is never exposed.
Keep both terminals running and keep the laptop awake. Laptop shutdown or loss of
internet makes the app unavailable. This repository does not install ngrok or
configure an ngrok account on your behalf.

## Convenient Windows launcher

```powershell
.\scripts\start.ps1 -Mock
.\scripts\start.ps1 -ShowKey
.\scripts\start.ps1 -PublicOrigin https://your-domain.ngrok-free.app
```

The script builds the release binary, uses this repository's `data` directory,
and runs in the current terminal. It never opens a second visible window or
prints the key unless you explicitly pass `-ShowKey`.

## Configuration

Run `pocket-codex --help` for the complete CLI.

| Flag | Environment variable | Default |
| --- | --- | --- |
| `--bind` | `POCKET_BIND` | `127.0.0.1:8787`; loopback only |
| `--data` | `POCKET_DATA` | `data` relative to the current directory |
| `--public-origin` | `POCKET_PUBLIC_ORIGIN` | Local browser access only |
| `--codex-bin` | `POCKET_CODEX_BIN` | Desktop install detection or `codex` on PATH |
| `--codex-home` | `POCKET_CODEX_HOME` | CLI's normal configuration directory |
| `--model` | `POCKET_MODEL` | Your CLI's configured model |
| `--mock` | `POCKET_MOCK` | Off |
| `--turn-timeout-secs` | — | 900 seconds |
| `--show-key` | — | Print key and exit |

No `.env` file is automatically loaded. Set environment variables in your shell
or use explicit flags. For a packaged executable, use an **absolute** `--data`
path to keep the same history regardless of where you launch it.

The app uses your Codex login on the server. It never sends that login credential
to the browser. It does not guarantee image entitlement: a successful native
image-generation turn is the definitive check. It reports unsupported access or
usage-limit failures rather than substituting another image service. There is no
OpenAI API-key integration in this version.

## Where everything lives

```text
data/                         ignored by Git
  .access-key                 locally generated shared login key
  .server.lock                prevents two server instances writing here
  chats/
    <chat-uuid>/
      chat.json               messages, image metadata, jobs, Codex thread id
      media/
        <image-uuid>.png       uploaded or generated binary image
  workspaces/
    <chat-uuid>/               Codex working directory and copied references
```

The application stores metadata as JSON. Codex maintains its own runtime/thread
history in its configured home directory; this app does not change Codex's
internal storage format. Back up the app's `data` directory and your chosen
Codex home together if you want to resume the original model conversations.

An unfinished job becomes `interrupted` on restart. It is **not** automatically
replayed, because replaying a generation could spend usage twice. Previously
saved messages and images remain available. Streaming text is saved roughly once
a second and at item completion; a sudden crash can lose the last partial text.

## Boundaries

This is a private, single-household app, not a multi-tenant service. The shared
login sees all projects. Session tokens are random, kept in HttpOnly cookies,
hashed in server memory, and expire after seven days or on server restart.
Login attempts are rate limited globally; authenticated users share one queue.

Shell, unified execution, browser/computer control, apps, and multi-agent features
are disabled for the private Codex process. Unexpected approval or client-tool
requests are rejected. File responses use registered image IDs, and generated
saved paths are accepted only inside the chat's workspace (or through base64
image data).

**Those controls are not an OS isolation boundary.** A normal Codex home can
contain installed MCP servers, skills, hooks, and other inherited settings.
Only give access to a trusted person. For stronger isolation, use a dedicated
OS account and a fresh Codex home signed in for this purpose, then point
`--codex-home` at it. Do not run this app as Administrator/root.

Chat and image files are local, but prompts and references go to the configured
inference provider. ngrok carries remote browser traffic. Files are not encrypted
at rest by the app; protect the laptop account and use disk encryption if needed.

Limits are deliberate: 500 projects, 1,000 messages and 500 images per project,
five attached references, 12 MB per image, 16,000 bytes per prompt, 8 MB per chat
JSON file, one active worker, eight waiting slots. There is no automatic media
deletion, disk quota, scheduling service, or automatic startup in this version.

## Learn the code

Start with [the walkthrough](docs/WALKTHROUGH.md). It explains the request flow,
ownership, persistence, streaming, and the Codex protocol with concrete examples.
[The HTTP API](docs/API.md) documents the browser/server contract.

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

The tests cover persistence and process locking, concurrent JSON edits, restart
recovery, authentication and CSRF, private images, cancellation, mock image
delivery, and confinement of generated file paths. CI runs on Windows and Linux.

Rebuild after editing `public/`: the assets are embedded at compile time.

