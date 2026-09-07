# M12 — voice completion and daily-use reliability

Companion to `docs/M12-VOICE-AUDIT.md`, which records what the M10 code actually
did before any of this. Read that first; this document describes what changed
and what is still true.

## There are two voice paths, and only one is used

This is the single most important thing to know about voice in this repository,
and it is not obvious from the code.

**The path the app uses:**

```
HomeScreen (hold the orb)
  -> useVoiceInput          MediaRecorder, audio/webm;codecs=opus
  -> useVoiceTurn
     -> POST /v1/audio/transcribe     OpenAI Whisper
     -> executeTurn                   text WebSocket -> Assistant Core
     -> POST /v1/voice/speak          Cartesia TTS
     -> AudioPlaybackController
```

**The path that exists but nothing calls:** the `voice_*` WebSocket frames on
`WS /v1/conversation/:id/stream`, and `POST /v1/voice/transcribe`. The mobile
client never sends or handles a single voice frame. Both fabrications found in
the audit lived here, which is why they were never noticed.

Note the consequence: **the app's speech-to-text is OpenAI Whisper, not
Cartesia.** Cartesia currently serves text-to-speech only. The Cartesia STT
provider is real and works, but only the unused path reaches it.

Both paths are now correct. The unused one was fixed rather than deleted because
deleting a protocol surface is a bigger decision than M12 should make on its own.

## Voice state machine

`crates/assistant-voice/src/state_machine.rs` holds the transition table.
`VoiceState` now lives in `assistant-protocol`, so the server, the state machine
and the TypeScript client all name the same states — previously the machine
validated a parallel copy while the wire carried a hand-written `String`.

Valid transitions:

| From | To |
| --- | --- |
| Idle | Listening |
| Listening | Transcribing, Idle |
| Transcribing | Thinking, Idle |
| Thinking | Speaking, Listening, Idle |
| Speaking | Idle, Listening |
| any | Interrupted, Error |
| Interrupted | Idle |
| Error | Idle |

Two rules are enforced by test: an interrupted turn can never walk back into
`Speaking`, `Thinking` or `Transcribing`, and a turn can never jump straight to
`Speaking` without having transcribed and thought first.

## Turn identity and cancellation

Every turn has an id, on both sides.

**Client** (`apps/mobile/src/lib/useVoiceTurn.ts`): mints a `crypto.randomUUID()`
per turn and re-checks it after every await. A superseded turn cannot write
state and cannot play audio. Starting or interrupting a turn aborts the previous
turn's HTTP requests through an `AbortController`, so it stops costing time and
money rather than merely being ignored.

**Server** (`routes/conversation.rs`): tracks the active `VoiceTurnId` and its
`CancellationToken`. Frames carrying any other turn id are dropped. `VoiceStart`
supersedes and cancels the previous turn; `VoiceCancel`/`VoiceInterrupted` are
honoured only from the turn that owns them, and actually cancel — the token
propagates into the model call and the provider requests.

`PROTOCOL_VERSION` is 10. Every voice frame carries `turn_id`.

## Failure behaviour

Errors are classified rather than collapsed. `VoiceError::code()` produces a
stable discriminant the client branches on:

| Code | Meaning | Retryable |
| --- | --- | --- |
| `voice_unconfigured` | server has no provider configured | no |
| `voice_provider_auth_failed` | provider rejected the server's key | no |
| `voice_network_error` | transport failure | yes |
| `voice_timeout` | exceeded the 30 s ceiling | yes |
| `voice_provider_error` | provider returned an error | yes |
| `voice_audio_encoding_error` | audio could not be encoded | no |
| `voice_cancelled` | superseded or interrupted | no |

The client maps these to specific messages — an expired session says so, an
unreachable server says so, and silence says "I didn't catch that". Nothing is
`console.warn` only any more; before this every voice failure was invisible and
a failed turn looked exactly like the assistant choosing not to answer.

Deliberate rule: a failed synthesis does **not** invalidate the answer. The text
reply is already on screen, so TTS failure reports that the reply was not spoken
rather than discarding it.

## Timeouts and limits

| Thing | Limit | Was |
| --- | --- | --- |
| Cartesia STT / TTS call | 30 s | none — `reqwest::Client::new()` |
| Client transcribe / speak request | 30 s | none |
| Uploaded audio | 10 MB | none |
| Synthesis text | 4000 chars | none |
| WebSocket audio frame | 14 MB encoded | none |
| Minimum recording | 200 bytes | same, but now reported |

While adding these it emerged that axum's **default 2 MB body limit had never
been overridden**, so the handler's own check was unreachable and an ordinary
recording could be refused by the framework with no explanatory code. The
transcribe route now sets an explicit limit just above the handler's.

## Microphone

Capture failures are classified (`denied`, `unavailable`, `unsupported`) and
shown. Previously a denied microphone fell into an empty `catch {}` and then ran
a **synthetic waveform**, so the orb animated as though it were hearing the user.
That was the app lying about whether it was listening, and it is gone.

Android runtime permission is not requested explicitly — the WebView prompts on
first `getUserMedia`. Denial is now surfaced with a recovery instruction rather
than being retried in a loop.

## Lifecycle

Leaving the screen or backgrounding the app abandons the current turn, aborts
its requests and stops playback. Previously the playback controller was never
stopped on unmount, so a reply kept talking over whatever the user switched to.

`AudioPlaybackController.stop()` now also **settles** the promise from the clip
it interrupted. It used to only pause the element, so an awaited `playBase64`
that got barged in on never resolved and its caller's `finally` never ran.

## Approvals

Voice cannot approve anything. A turn that reaches a tool requiring approval
stops at the policy exactly as the text path does — nothing runs — and the voice
layer now names the pending action and sends the user to the existing approval
UI. Previously `approval_required` frames were ignored, so the turn returned no
answer and fell silent, indistinguishable from being ignored.

This is deliberate: "yes" is far too easy to say by accident, and far too easy
to mishear, for an action that sends mail or changes a calendar. There is no
spoken consent path, and the existing approval mechanism remains the only way to
authorise a held action.

## Conversation continuity

Voice turns share one conversation for the life of the screen, so a follow-up
carries the previous turns' context. Each turn previously minted a fresh
conversation id, so "which one is due first?" arrived with no history and could
not be answered. `resetConversation()` starts a new one deliberately.

## Measured latency

From the live Cartesia round trip (`cargo test -p assistant-voice --test
live_cartesia -- --ignored`), against the real API:

| Stage | Measured |
| --- | --- |
| Cartesia TTS (`sonic-3.6`, 77 812 bytes WAV) | **906.95 ms** |
| Cartesia STT (`ink-whisper`) | **204.42 ms** |

These are provider round trips only. The full pipeline additionally includes
Whisper transcription and the model turn, which were not measured end to end on
device — see limitations. No end-to-end latency claim is made here.

Two structural costs are visible in the code and not yet addressed: providers
are constructed per request, so every call builds a fresh `reqwest::Client` and
a new TLS connection; and the server hardcodes `sample_rate: 24000` regardless
of what the client actually recorded.

## Offline

Voice does not work offline and does not pretend to. Capture is local, but
transcription, the assistant turn and synthesis all require the server. With no
network the client reports `voice_network_error` ("Couldn't reach the server")
and returns to idle. Nothing is queued and no turn is fabricated.

## Security

- All three voice routes sit behind `auth::require_bearer`; unauthenticated and
  forged-token requests are rejected. Covered by test.
- No voice route accepts a user or session id from the request body — ownership
  comes from the authenticated principal or the URL path.
- No provider credential reaches the mobile bundle. Verified by grep across
  `apps/mobile`: only `VITE_SERVER_BASE_URL` and `VITE_DEV_AUTH_TOKEN` are read.
- `transcribe.rs` no longer logs the provider response body.
- No audio bytes, transcripts or keys are logged anywhere in the voice path.
- The diagnostic endpoint reports whether a key is configured, never its value.
  Covered by test.

Voice gains the model no additional authority: a voice turn builds the same
`TurnRequest` and runs through the same `run_turn`, so the same permission
policy, approval requirements and tool executor apply.

## Known limitations

- **Not verified on the device end to end.** See the verification section of the
  final report for exactly what was and was not exercised on hardware.
- **Streaming is not implemented, and is not faked.** Cartesia's documented STT
  path for this integration is the batch `POST /stt` endpoint; TTS `/tts/bytes`
  returns a single response. Both are buffered, which is honest for these
  endpoints. Nothing pretends to stream.
- **Haptics remain inert** (see `docs/M11-INTERACTION.md`).
- **Providers are constructed per request** rather than reused.
- **Rate limiting is per-instance.** A fixed-window limiter (30 calls per
  principal per minute, per endpoint) lives in process memory, so it becomes
  per-instance if the server is ever replicated. Adequate to stop a runaway
  client; not a distributed quota. A shared limiter belongs with the rest of the
  M13 abuse surface.
- **The unused WebSocket voice path is still present.** It is now correct, but
  it is dead weight until either the client adopts it or it is removed.
- **`sample_rate` is hardcoded** to 24000 on the server regardless of the
  client's actual recording.
