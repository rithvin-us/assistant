# Domain Vocabulary & Conceptual Boundaries

This document defines the core domain concepts required for the voice-first personal operating system and AI assistant.

---

## 1. Core Domain vs. Integration Scope

| Domain Concept | Crate Ownership | Primary Purpose |
| :--- | :--- | :--- |
| **`User`** | `assistant-auth` / `assistant-core` | Primary owner of the assistant instance and data. |
| **`Account`** | `assistant-auth` / `assistant-tools` | Connected external identity (e.g. Personal Google, College Google, Work Google). |
| **`Conversation`** | `assistant-protocol` / `assistant-core` | Interactive dialogue session between user and assistant. |
| **`Message`** | `assistant-protocol` / `assistant-core` | Single turn within a conversation (User text, Assistant delta, Tool call/result). |
| **`Task`** | `assistant-core` | Actionable item with priority, state (`Pending`, `Completed`), and optional project binding. |
| **`Deadline`** | `assistant-core` | Hard time constraint associated with a task, project, or schedule slot. |
| **`Reminder`** | `assistant-core` | Time-triggered or context-triggered notification prompt for the user. |
| **`Event`** | `assistant-core` / `assistant-tools` | Calendar entry with start/end timestamps, location, and account binding. |
| **`ScheduleSlot`** | `assistant-core` | Calculated block of allocated or free time in the user's daily timeline. |
| **`Project`** | `assistant-core` | Goal-oriented grouping of tasks, deadlines, documents, and ideas. |
| **`ProjectItem`** | `assistant-core` | Element within a project hierarchy. |
| **`Person`** | `assistant-core` | Contact or entity referenced across conversations, emails, or events. |
| **`Idea`** | `assistant-core` / `assistant-memory` | Uncommitted thought captured for later review or promotion. |
| **`Memory`** | `assistant-memory` | Attributed, promoted claim with lifecycle, importance, confidence, and provenance. |
| **`Document`** | `assistant-tools` / `assistant-core` | Ingested file (PDF, image, text) with page-level provenance and extracted facts. |
| **`Email`** | `assistant-tools` (Integration) | Message received, searched, or drafted across connected Google accounts. |
| **`Notification`** | `assistant-core` | Proactive alert scored by the attention engine for the user. |
| **`Watchdog`** | `assistant-core` | Background rule checking deadlines, unread priority mail, or schedule conflicts. |
| **`Approval`** | `assistant-tools` | Human-in-the-loop permission request for `Orange` and `Red` risk tools. |
| **`ToolExecution`** | `assistant-tools` / `assistant-core` | Audit log record of a tool execution attempt, decision, result, and latency. |
| **`Automation`** | `assistant-core` | Programmed background rule or recurring workflow. |
| **`Source`** | `assistant-core` / `assistant-tools` | Data origin (Gmail, Calendar, Drive, Web, Manual capture, Desktop agent). |
| **`AuditEvent`** | `assistant-core` / `assistant-auth` | Immutable security log entry for sensitive identity or tool operations. |

---

## 2. Orchestration Pipeline Flow

Every user input follows this deterministic pipeline:

```text
User Input (Voice / Text)
  │
  ▼
1. Normalize Input (Text / Audio decoding)
  │
  ▼
2. Intent Classification (Deterministic SQL lookup vs. Cheap Model Router)
  │
  ▼
3. Context Assembly (Active conversation state + Relevant Memories + Schedule)
  │
  ▼
4. Decision & Routing (Select Model & Tool Candidates)
  │
  ▼
5. Permission Evaluation (ToolSpec RiskLevel vs. Deterministic Policy)
     ├── Allow           ➜ Execute Tool
     ├── RequireApproval ➜ Create Persistent Approval (Requested)
     └── Deny            ➜ Reject & Audit
  │
  ▼
6. Tool Execution & Verification
  │
  ▼
7. Response Generation & Token Streaming (ServerFrame::AssistantDelta ➜ TurnEnd)
```

---

## 3. Account & Multi-Google Identity Support

The domain supports arbitrary connected accounts per user without hard-coding specific accounts:

```text
User (ID)
  ├── Account 1: Personal Gmail / Calendar (OAuth Scopes)
  ├── Account 2: College Gmail / Calendar (OAuth Scopes)
  ├── Account 3: Work Gmail / Calendar (OAuth Scopes)
  └── Account N: Connected Service (OAuth Scopes)
```

OAuth client secrets and provider API keys live strictly on the server side and are never exposed in mobile bundles or `assistant-core`.
