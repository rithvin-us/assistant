# Milestone 5 — Google Ecosystem + Personal Schedule Foundation

Milestone 5 connects the assistant to the user's real Google ecosystem (Gmail, Google Calendar) and establishes the foundation for personal schedule coordination.

At the core of this milestone is the principle of **Multi-Account Ownership & Model Independence**:
1. Connect an arbitrary number ($N$) of Google accounts (Personal, College, Work).
2. Strict isolation at the query and tool boundary (`user_id` + `account_id`).
3. Zero credentials on client or in React: OAuth refresh tokens encrypted at rest via AES-256-GCM.
4. Provider-neutral abstractions: `GmailProvider` and `CalendarProvider` decouple Google APIs from `assistant-core`.
5. Deterministic capabilities first: Manual UI for Connections, Gmail search/reading, Calendar agenda, and Free-Time slot arithmetic without requiring an LLM.

---

## 1. Google Authentication Architecture

### Secure OAuth Flow

```
Mobile Client
    │  1. Request OAuth start (GET /api/google/oauth/start)
    ▼
Assistant Server
    │  2. Generate CSRF state token and Google consent URL with offline access
    ▼
Mobile Browser / In-App Custom Tab
    │  3. User consents to minimal scopes
    ▼
Assistant Server Callback (GET /api/google/oauth/callback or POST /api/google/oauth/exchange)
    │  4. Server exchanges authorization code for tokens directly with Google OAuth2
    │  5. Access token (temporary) + Refresh token (encrypted at rest via AES-256-GCM)
    │  6. Store sanitized record in PostgreSQL `connected_accounts` table
    ▼
Mobile Client
    │  7. Receives sanitized `AccountSummary` (id, email, status, scopes)
    ▼
```

### Encrypted Credential Storage (ADR-0025)
- Stored in `connected_accounts` table in PostgreSQL.
- Credentials JSON (`{"refresh_token": "...", "access_token": "...", "expires_at": ...}`) is encrypted using AES-256-GCM with a 12-byte cryptographically random initialization vector (IV) prepended to the ciphertext.
- Server decrypts credentials in-memory only when actively executing an API call.
- Transparent token refresh handles Google access token expiry without mobile interaction.

---

## 2. Multi-Account Isolation & Query-Level Enforcement (ADR-0026)

Every Google operation requires an explicit `account_id` alongside the authenticated `user_id`.

```sql
SELECT * FROM connected_accounts 
WHERE id = $1 AND user_id = $2 AND status = 'active';
```

- If `user_id` does not match the account owner, the operation returns `Permission Denied` (404/403).
- Disconnecting an account updates `status = 'disconnected'` and invalidates credentials immediately without deleting historical conversation logs.

---

## 3. Minimal Scopes

We request only the minimum required OAuth scopes:
- `https://www.googleapis.com/auth/userinfo.email`
- `https://www.googleapis.com/auth/userinfo.profile`
- `https://www.googleapis.com/auth/gmail.readonly` (Search and read emails; no send or modify permissions)
- `https://www.googleapis.com/auth/calendar` (Read, create, update, delete calendar events)

---

## 4. Tool Registry & Risk Policy Integration

Google capabilities are registered in `ToolRegistry` with immutable static risk levels (ADR-0005):

| Tool | Risk Level | Policy / Execution Flow |
|---|---|---|
| `gmail.search` | **Green** | Automated / Read-only |
| `gmail.read` | **Green** | Automated / Read-only |
| `calendar.list` | **Green** | Automated / Read-only |
| `calendar.search` | **Green** | Automated / Read-only |
| `calendar.free_slots` | **Green** | Automated / Read-only |
| `calendar.create` | **Yellow** | Automated low-risk write |
| `calendar.update` | **Yellow** | Automated low-risk write |
| `calendar.delete` | **Orange** | **Requires Explicit Human Approval via Durable Actions** |

---

## 5. Deterministic Free-Time Engine (ADR-0027)

Deterministic interval arithmetic computes available free-time windows given:
- Start and end datetime boundaries.
- Busy calendar events.
- Minimum task duration (in minutes).

Algorithm:
1. Sort all calendar events by start time.
2. Merge overlapping or contiguous events.
3. Compute gaps between the search window start, events, and search window end.
4. Filter gaps that are $\ge$ requested duration.

No LLM or heuristic guess is involved in scheduling.

---

## 6. Frontend & Mobile UI

Built with React 19 + Material UI, following the Todoist-inspired Pure Light Theme:
- **Connections Screen**: View all connected Google accounts (Personal, College, Work), connect new accounts via OAuth, and disconnect/revoke accounts.
- **Gmail Screen**: Search emails with native Gmail syntax (`is:unread`, `from:`, `subject:`), switch accounts via tabs, and view email threads with clean privacy sanitization.
- **Calendar Screen**: View Today, Upcoming agenda, and Free Time slots; switch between connected accounts; manually create events with title, date, time, and location.

---

## 7. Privacy & Logging Standards

- No authorization tokens, client secrets, or refresh tokens are ever printed in logs.
- Full email message bodies are never written to server application logs.
- Email contents are fetched on-demand and not permanently retained in PostgreSQL.
