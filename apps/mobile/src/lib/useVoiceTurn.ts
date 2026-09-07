/**
 * useVoiceTurn — owns one voice turn at a time, honestly.
 *
 * A voice turn is capture -> transcribe -> assistant -> speak, four awaits deep,
 * any of which can outlive the turn that started it. Before this, nothing tied
 * those steps together: a transcript that arrived after the user had already
 * started asking something else was applied to the new turn, and a TTS clip from
 * an abandoned turn played over the next question.
 *
 * Two rules make that impossible:
 *
 *   1. Every turn has an id. After each await the turn checks whether it is
 *      still the current one and abandons itself if not. A superseded turn can
 *      never write state or play audio.
 *   2. Starting or cancelling a turn aborts the previous one's in-flight
 *      requests, so it stops costing time and money rather than merely being
 *      ignored.
 *
 * It also refuses to invent anything. Silence is reported as silence, a failed
 * transcription as a failed transcription, and a failed synthesis does not
 * pretend the reply was spoken — the text answer still stands.
 */

import { useCallback, useEffect, useRef, useState } from "react";

import { AudioPlaybackController, VoiceRequestError, speakVoiceText } from "../api/voice";
import { transcribeAudio } from "../api/transcribe";
import { executeTurn } from "../api/conversation";
import type { VoiceState } from "../api/types";

/**
 * Recordings shorter than this are treated as an accidental tap rather than
 * speech. Sending them wastes a provider call and usually transcribes to noise.
 */
const MIN_AUDIO_BYTES = 200;

export interface VoiceTurnFailure {
  /** Machine-readable, from the server where available. */
  code: string;
  /** Safe to show the user. Never contains provider detail or secrets. */
  message: string;
}

export interface VoiceTurn {
  state: VoiceState;
  /** What the user said, then what the assistant replied. Null between turns. */
  transcript: string | null;
  /** Set when the turn failed. Cleared when a new turn starts. */
  error: VoiceTurnFailure | null;
  /** Runs a turn from captured audio. Resolves when it settles, never throws. */
  run: (audio: Blob | null) => Promise<void>;
  /** Barge-in: stop speaking, abort in-flight work, return to idle. */
  interrupt: () => void;
  clearError: () => void;
}

/** Turns whatever was thrown into something safe and specific to show. */
function describe(err: unknown): VoiceTurnFailure {
  if (err instanceof VoiceRequestError) {
    switch (err.code) {
      case "unauthorized":
        return { code: err.code, message: "Your session expired. Sign in again." };
      case "voice_timeout":
        return { code: err.code, message: "The voice service timed out." };
      case "voice_network_error":
        return { code: err.code, message: "Couldn't reach the server." };
      case "voice_unconfigured":
        return { code: err.code, message: "Voice isn't configured on the server." };
      case "voice_provider_auth_failed":
        return { code: err.code, message: "The voice provider rejected the server's key." };
      default:
        return { code: err.code, message: err.message };
    }
  }
  if (err instanceof Error) return { code: "voice_error", message: err.message };
  return { code: "voice_error", message: "Something went wrong." };
}

export function useVoiceTurn(): VoiceTurn {
  const [state, setState] = useState<VoiceState>("idle");
  const [transcript, setTranscript] = useState<string | null>(null);
  const [error, setError] = useState<VoiceTurnFailure | null>(null);

  /** The turn allowed to write state. Anything else is stale by definition. */
  const currentTurn = useRef<string | null>(null);
  const abort = useRef<AbortController | null>(null);
  const playback = useRef(new AudioPlaybackController());
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    const player = playback.current;
    return () => {
      mounted.current = false;
      // Leaving the screen must not leave the assistant talking to an empty room.
      currentTurn.current = null;
      abort.current?.abort();
      player.stop();
    };
  }, []);

  // Backgrounding the app should stop playback too: Android keeps the WebView
  // alive, so without this the reply keeps playing over whatever the user
  // switched to.
  useEffect(() => {
    const onHidden = () => {
      if (document.visibilityState === "hidden") {
        currentTurn.current = null;
        abort.current?.abort();
        playback.current.stop();
        setState("idle");
      }
    };
    document.addEventListener("visibilitychange", onHidden);
    return () => document.removeEventListener("visibilitychange", onHidden);
  }, []);

  const interrupt = useCallback(() => {
    currentTurn.current = null;
    abort.current?.abort();
    abort.current = null;
    playback.current.stop();
    setState("idle");
    setTranscript(null);
  }, []);

  const clearError = useCallback(() => setError(null), []);

  const run = useCallback(async (audio: Blob | null) => {
    // Supersede whatever came before, and stop it costing anything.
    abort.current?.abort();
    playback.current.stop();

    const turnId = crypto.randomUUID();
    currentTurn.current = turnId;
    const controller = new AbortController();
    abort.current = controller;

    /** True while this turn is still the one the user is waiting on. */
    const live = () => mounted.current && currentTurn.current === turnId;

    setError(null);

    if (!audio || audio.size <= MIN_AUDIO_BYTES) {
      // Nothing was said. Saying so beats silently returning to idle, which
      // looks identical to the app ignoring you.
      if (live()) {
        setState("idle");
        setError({ code: "voice_no_speech", message: "I didn't catch that." });
      }
      return;
    }

    try {
      if (!live()) return;
      setState("transcribing");
      const spoken = await transcribeAudio(audio, controller.signal);
      if (!live()) return;

      if (!spoken || spoken.trim().length === 0) {
        setState("idle");
        setError({ code: "voice_no_speech", message: "I didn't catch that." });
        return;
      }

      setTranscript(spoken);
      setState("thinking");
      const reply = await executeTurn(spoken);
      if (!live()) return;

      if (!reply || reply.trim().length === 0) {
        setState("idle");
        setTranscript(null);
        return;
      }

      setTranscript(reply);

      // The answer exists and is already on screen. Speaking it is a separate
      // step that may fail on its own without invalidating the answer.
      setState("speaking");
      let audioBase64: string | null = null;
      try {
        const spokenReply = await speakVoiceText(reply, undefined, controller.signal);
        audioBase64 = spokenReply.audio_base64 ?? null;
      } catch (ttsErr) {
        if (!live()) return;
        setError(describe(ttsErr));
      }

      if (!live()) return;
      if (audioBase64) {
        await playback.current.playBase64(audioBase64);
      }
      if (!live()) return;
      setState("idle");
      setTranscript(null);
    } catch (err) {
      if (!live()) return;
      setState("error");
      setError(describe(err));
      setTranscript(null);
    } finally {
      if (live()) {
        currentTurn.current = null;
        abort.current = null;
      }
    }
  }, []);

  return { state, transcript, error, run, interrupt, clearError };
}
