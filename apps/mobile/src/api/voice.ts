/**
 * Voice API Client & WebAudio helper methods for M10.
 */

import { SERVER_BASE_URL, DEV_TOKEN } from './bridge';

export interface TranscribeResponse {
  text: string;
}

export interface SpeakResponse {
  audio_base64: string;
  encoding: string;
}

export interface VoiceDiagnosticResponse {
  cartesia_configured: boolean;
  stt_model: string;
  tts_model: string;
  tts_voice_id: string;
  status: string;
}

const BASE = () => `${SERVER_BASE_URL.replace(/\/+$/, '')}/v1/voice`;

/**
 * Ceiling on a single voice request.
 *
 * These were plain fetches with no timeout, so a stalled provider left the user
 * holding a phone that says "thinking" forever with no way back.
 */
const VOICE_REQUEST_TIMEOUT_MS = 30_000;

/** A failure with the server's machine-readable code preserved. */
export class VoiceRequestError extends Error {
  constructor(
    message: string,
    /** e.g. `voice_provider_auth_failed`, `voice_timeout`. */
    readonly code: string,
    readonly status?: number,
  ) {
    super(message);
    this.name = 'VoiceRequestError';
  }
}

function authHeaders(): HeadersInit {
  return {
    Authorization: `Bearer ${DEV_TOKEN}`,
  };
}

async function request<T>(
  url: string,
  init?: RequestInit,
  /** Caller's cancellation, e.g. the turn being superseded. */
  signal?: AbortSignal,
): Promise<T> {
  // Two reasons to give up: the caller cancelled, or we ran out of patience.
  // Both abort the same underlying request.
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(new DOMException('timeout', 'TimeoutError')), VOICE_REQUEST_TIMEOUT_MS);
  const onAbort = () => controller.abort(signal?.reason);
  signal?.addEventListener('abort', onAbort, { once: true });

  let res: Response;
  try {
    res = await fetch(url, {
      ...init,
      signal: controller.signal,
      headers: {
        ...authHeaders(),
        ...init?.headers,
      },
    });
  } catch {
    if (controller.signal.aborted) {
      const timedOut = (controller.signal.reason as DOMException)?.name === 'TimeoutError';
      throw new VoiceRequestError(
        timedOut ? 'The voice service did not respond in time.' : 'Cancelled.',
        timedOut ? 'voice_timeout' : 'voice_cancelled',
      );
    }
    throw new VoiceRequestError('Could not reach the server.', 'voice_network_error');
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener('abort', onAbort);
  }

  if (!res.ok) {
    const err = await res.json().catch(() => ({ message: res.statusText, code: undefined }));
    // Keep the server's code. Collapsing everything to one message is what left
    // the UI unable to tell an expired session from a provider outage.
    throw new VoiceRequestError(
      err.message || `Request failed with status ${res.status}`,
      err.code ?? (res.status === 401 ? 'unauthorized' : 'voice_provider_error'),
      res.status,
    );
  }

  return res.json() as Promise<T>;
}

export async function transcribeVoiceAudio(
  audioBlob: Blob,
  signal?: AbortSignal,
): Promise<TranscribeResponse> {
  const arrayBuffer = await audioBlob.arrayBuffer();
  return request<TranscribeResponse>(
    `${BASE()}/transcribe`,
    {
      method: 'POST',
      headers: {
        // The recorder's real type. Declaring audio/wav for what is actually
        // webm/opus makes the server guess wrong.
        'Content-Type': audioBlob.type || 'audio/webm',
      },
      body: arrayBuffer,
    },
    signal,
  );
}

export async function speakVoiceText(
  text: string,
  voiceId?: string,
  signal?: AbortSignal,
): Promise<SpeakResponse> {
  return request<SpeakResponse>(
    `${BASE()}/speak`,
    {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ text, voice_id: voiceId }),
    },
    signal,
  );
}

export async function getVoiceDiagnostic(): Promise<VoiceDiagnosticResponse> {
  return request<VoiceDiagnosticResponse>(`${BASE()}/diagnostic`, {
    method: 'GET',
  });
}

/**
 * Plays one clip at a time.
 *
 * There is exactly one owner of playback per controller, and starting a clip
 * stops whatever was playing. Crucially `stop()` also SETTLES the promise from
 * the clip it interrupted: it previously only paused the element, so an awaited
 * `playBase64` that got barged in on never resolved or rejected, and the caller
 * sat in its `try` forever with the `finally` never running.
 */
export class AudioPlaybackController {
  private currentAudio: HTMLAudioElement | null = null;
  /** Settles the in-flight playBase64 when playback is cut short. */
  private finishCurrent: ((outcome: 'ended' | 'stopped') => void) | null = null;

  /**
   * Resolves when the clip finishes, or when it is stopped. Being interrupted
   * is a normal outcome of barge-in, not an error, so it resolves rather than
   * throwing; genuine playback failures still reject.
   */
  playBase64(base64Data: string, mimeType: string = 'audio/wav'): Promise<'ended' | 'stopped'> {
    this.stop();
    return new Promise((resolve, reject) => {
      try {
        const audio = new Audio(`data:${mimeType};base64,${base64Data}`);
        this.currentAudio = audio;

        let settled = false;
        const settle = (outcome: 'ended' | 'stopped') => {
          if (settled) return;
          settled = true;
          this.finishCurrent = null;
          resolve(outcome);
        };
        this.finishCurrent = settle;

        audio.onended = () => {
          if (this.currentAudio === audio) this.currentAudio = null;
          settle('ended');
        };
        audio.onerror = () => {
          if (this.currentAudio === audio) this.currentAudio = null;
          if (settled) return;
          settled = true;
          this.finishCurrent = null;
          reject(new Error('Could not play the spoken reply.'));
        };
        audio.play().catch((err) => {
          if (settled) return;
          settled = true;
          this.finishCurrent = null;
          reject(err);
        });
      } catch (err) {
        reject(err);
      }
    });
  }

  /** True while a clip is actually playing. */
  get isPlaying(): boolean {
    return this.currentAudio !== null;
  }

  stop() {
    const audio = this.currentAudio;
    this.currentAudio = null;
    if (audio) {
      audio.pause();
      audio.currentTime = 0;
      // Release the element's decoder rather than leaving it to the GC.
      audio.src = '';
    }
    this.finishCurrent?.('stopped');
  }
}
