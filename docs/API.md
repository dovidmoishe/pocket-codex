# HTTP API

All routes are same-origin. Private routes require the `pocket_session` cookie.
Every mutating request also requires `X-Pocket-Request: 1` and an allowed `Origin`.
The browser sets Origin automatically. Public deployments must configure
`--public-origin`; loopback requests are accepted by default.

| Method | Path | Body / response |
| --- | --- | --- |
| POST | `/api/login` | `{ "key": "your-long-access-key" }`; sets HttpOnly cookie |
| POST | `/api/logout` | Revokes this session and clears cookie |
| GET | `/api/status` | Version and Codex/mock readiness, image capability, error |
| GET | `/api/chats` | Project summaries, newest first |
| POST | `/api/chats` | `{ "title": "My project" }`; 201 with full chat |
| GET | `/api/chats/{id}` | Full saved chat JSON |
| PATCH | `/api/chats/{id}` | `{ "title": "Updated title" }` |
| GET | `/api/chats/{id}/export` | Downloadable chat JSON |
| POST | `/api/chats/{id}/messages` | Prompt below; 202 with queued job |
| POST | `/api/chats/{id}/uploads` | Multipart: exactly one `file`; 201 with image metadata |
| POST | `/api/chats/{id}/jobs/{job_id}/cancel` | Request cancellation |
| GET | `/api/chats/{id}/media/{media_id}` | Authenticated image stream |
| GET | `/api/chats/{id}/media/{media_id}?download=1` | Attachment download |
| GET | `/api/events` | Authenticated SSE, event name `update` |

Prompt:

```json
{
  "text": "Make a minimalist birthday invitation in peach and green",
  "mode": "generate",
  "media_ids": []
}
```

Mode is `chat` or `generate`. Media IDs must belong to the same project; up to five
are allowed. A 202 response means queued, **not** successfully generated. Check
the job's saved status or listen to events. Duplicate submissions are not
automatically retried by the frontend. The server rejects submissions while a
project already has active/queued work.

SSE examples:

```text
event: update
data: {"type":"refresh","chat_id":"<uuid>"}

event: update
data: {"type":"text","chat_id":"<uuid>","message_id":"<uuid>","job_id":"<uuid>","text":"The current assembled reply"}

event: update
data: {"type":"progress","chat_id":"<uuid>","text":"Generating your image…"}

event: update
data: {"type":"resync"}
```

`text` is a current snapshot, not a delta to append. `refresh` means reload project
metadata/history. `status` means refresh Codex status. `resync` means reload
snapshots, including after reconnect or event-buffer overflow. SSE is live-only;
there is no durable event replay log. Persisted JSON is the source of truth.

Errors generally use `{ "error": "a useful message" }`. Auth: 401. Origin
rejection: 403. Missing resource: 404. Busy/full queue: 409. Login rate limit: 429.
Axum's malformed JSON/multipart and body-limit rejections may be plain-text.

