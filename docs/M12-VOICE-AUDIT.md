# M12 — audit of the M10 voice implementation

Written before any M12 code change. Everything below was read in the current
tree or verified against the live Cartesia documentation; nothing is inferred
from the M10 specification. Where a type exists but nothing uses it, that is
recorded as unused rather than implemented.

## Verdict

The voice path **cannot currently work against real Cartesia**, and if it did,
it would speak a sentence unrelated to the assistant's answer. Two of the
defects below are fabrications of the kind CLAUDE.md forbids outright.

## Classification

### D. Incorrect — these are the ones that matter

**D1. The assistant speaks a hardcoded sentence, not its answer.**
`services/assistant-server/src/routes/conversation.rs:277` synthesises the
literal string `"I checked your schedule and tasks for today."` for every voice
turn. `run_turn` streams the model's real reply as text frames, and that text is
never captured for TTS. So the spoken output is disconnected from the actual
answer — the user hears a canned line whatever they asked. Directly violates
"Never fabricate an assistant response."

**D2. A fabricated transcript is fed into a real turn.**
`services/assistant-server/src/routes/conversation.rs:233`: when
`cartesia_api_key` is absent, the transcript falls back to
`"What should I do today?"` and the server then runs a genuine turn on it. That
is an invented user utterance driving real tool execution.

**D3. The Cartesia STT model id is wrong.** Code default `ink-en-us`
(`cartesia_stt.rs:29`) and config default `ink-en-us` (`config.rs:161`). The
API requires the `ink-whisper` family; `ink-en-us` is not a valid model.

**D4. The Cartesia TTS model id is wrong.** Code default `sonic-2`
(`cartesia_tts.rs:41`), config default `sonic-english` (`config.rs:162`).
Current valid ids are `sonic-3.6`, `sonic-3.5`, `sonic-3`, `sonic-latest`.
`sonic-english` is not among them.

**D5. The API version header is two years stale.** Both providers send
`Cartesia-Version: 2024-06-10` (`cartesia_stt.rs:83`, `cartesia_tts.rs:99`).
The documented value is `2026-08-14`.

D3–D5 together mean every real Cartesia call is rejected, which is what the
hardcoded fallbacks in D1/D2 were papering over.

**D6. TTS failure is silently swallowed.** `conversation.rs:284` is
`if let Ok(audio) = tts.synthesize(req).await` — a failed synthesis sends no
frame at all. The client sees `speaking` then `idle` and hears nothing, with no
error.

**D7. Two independent playback owners.** `HomeScreen.tsx:60` and
`VoiceController.tsx:26` each construct their own `AudioPlaybackController`.
Neither knows about the other, so both can play at once.

**D8. Blob type is hardcoded.** `VoiceController.tsx:44` builds the blob as
`audio/wav` regardless of what `MediaRecorder` actually produced (which is
`audio/webm;codecs=opus` on Android). The declared type does not match the
bytes.

**D9. Server ignores the real sample rate.** `conversation.rs` hardcodes
`sample_rate: 24000` for whatever the client sent.

### C. Stubbed

- `conversation.rs:159` — `VoiceStart` only emits `state: "listening"`. Nothing starts.
- `conversation.rs:169` — `VoiceCancel` / `VoiceInterrupted` only emit `state: "interrupted"`. **No in-flight STT, model or TTS work is cancelled.** Interruption is cosmetic server-side.
- `routes/voice.rs:168` — `/v1/voice/diagnostic` reports whether config keys exist. It never contacts the provider, so it cannot tell you the credentials or model ids are wrong.
- `useVoiceInput.ts:147` — empty `catch {}` around `getUserMedia`; a denied microphone silently falls through to a synthetic waveform animation, so the UI *looks* like it is listening.

### E. Missing

**E1. No turn identity anywhere.** No voice frame carries a turn or request id —
`VoiceStateChanged`, `VoiceTranscriptFinal`, `VoiceTtsChunk`, `VoiceEnd` all
lack one (`assistant-protocol/src/lib.rs:116-134`), and the client never sends
one. A late result from turn A cannot be distinguished from turn B. This is the
single largest reliability gap and the direct cause of the stale-audio class of
bug.

**E2. No timeouts on voice providers.** Both providers use
`reqwest::Client::new()` (`cartesia_stt.rs:27`, `cartesia_tts.rs:39`) with no
timeout, so a hung STT or TTS call hangs forever. `model_timeout` (60s) exists
for inference only and is not applied to voice.

**E3. No cancellation into the voice path.** `socket_cancel` exists and is
passed to `run_turn`, but not to the STT or TTS calls in the
`VoiceAudioChunk` handler.

**E4. No limits.** No body-size cap, audio-duration cap, concurrent-session cap
or rate limit on any voice route or on `VoiceAudioChunk`.

**E5. No microphone permission handling.** No `permissions.query`, no Android
runtime request, no recovery path. Denial is indistinguishable from silence.

**E6. No client-side abort.** `transcribeVoiceAudio`, `speakVoiceText` and
`transcribeAudio` are plain `fetch` calls with no `AbortController` and no
timeout.

**E7. No lifecycle cleanup.** No `visibilitychange`/`pagehide` handling;
`HomeScreen`'s playback controller is never stopped on unmount, so TTS keeps
playing after the screen goes away.

**E8. Voice errors are not classified.** Everything collapses to `stt_error` /
`tts_error` with a bare `err.to_string()`. Auth failure, network failure,
timeout and provider rejection are indistinguishable to the client.

### B. Partially implemented

- The `VoiceError` enum does classify failures (`Unconfigured`, `AuthenticationFailed`, `Network`, `Api`) — but that structure is flattened at the API boundary (E8).
- `run_turn` is genuinely shared with the text path, so §8 ("voice must use the same Assistant Core") holds today. Voice does **not** have a second brain — this is correct and must be preserved.
- STT has a real, undocumented OpenAI fallback (`cartesia_stt.rs:96-110`) that fires on any non-success Cartesia response.

### A. Actually implemented and correct

- Voice HTTP routes sit behind `auth::require_bearer` (`routes/mod.rs:174-177`).
- No voice route accepts a user/session id from the request body — ownership is taken from the authenticated principal or the URL path. No privilege-escalation surface found.
- **No provider credential reaches the mobile bundle.** Grep for `CARTESIA`/`api_key` across `apps/mobile` returns nothing; only `VITE_SERVER_BASE_URL` and `VITE_DEV_AUTH_TOKEN` are read. §31 holds.
- `VoiceStateMachine` (`state_machine.rs`) is correct, deterministic and well-tested — see the note below.
- Capture negotiates a real codec (`useVoiceInput.ts:85-91`) and cleans up its own stream on unmount (`useVoiceInput.ts:166-180`).
- Barge-in exists on the client: tapping during `thinking`/`speaking` stops playback (`HomeScreen.tsx:132`).

## The state machine is real but governs nothing

`crates/assistant-voice/src/state_machine.rs` is a correct, exhaustive,
deterministic transition table with `InvalidTransition` errors. It is referenced
**only by its own unit tests** in `lib.rs` — no route, no handler, no frame uses
it.

The wire protocol carries `VoiceStateChanged { state: String }` — a bare string —
and the server hand-writes `"listening"`, `"transcribing"`, `"thinking"`,
`"speaking"`, `"idle"` at seven call sites with no validation that the sequence
is legal. The client then keeps *two more* state variables: `activeVoiceState`
(`OrbState`) in `HomeScreen` and `voiceState` (`VoiceState`) in
`VoiceController`, updated from scattered handlers.

So there are three state representations and the only validated one is unused.

## Logging

One real leak: `routes/transcribe.rs:130` logs the full OpenAI response body
(`body = %text`). Otherwise voice logging is clean — `routes/voice.rs` has no
tracing at all, and the WebSocket handler logs only `user_id`. No audio bytes,
transcripts or keys are logged.

## Streaming

Cartesia's documented batch STT endpoint is `POST https://api.cartesia.ai/stt`.
No WebSocket STT endpoint appears in the current reference for this path.
Buffered is therefore the honest implementation, and per §5 it must be
documented as such rather than dressed up as streaming. TTS `/tts/bytes` is
likewise a single-response byte endpoint. The current code is buffered and does
not fake streaming — that part is correct and should stay.

## Physical device status

The app builds, installs and runs on the attached device (serial `1025f519`) as
of M11. The voice path has **not** been verified end to end on hardware, and
given D3–D5 it cannot have worked against real Cartesia at any point.

## Work order for M12

Ordered by dependency, not by section number:

1. Correct the provider contract (D3, D4, D5) — nothing else can be verified until real calls succeed.
2. Speak the real answer (D1) and delete the fabricated transcript (D2).
3. Introduce a turn id end to end (E1); make it the basis for rejecting stale results.
4. Make the Rust state machine the authority; carry a typed state on the wire.
5. Timeouts, cancellation, limits (E2, E3, E4).
6. Permissions, abort, lifecycle on the client (E5, E6, E7).
7. Error classification (E8), then the security and regression tests.
