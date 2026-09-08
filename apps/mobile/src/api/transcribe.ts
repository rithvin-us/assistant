import { getServerBaseUrl, DEV_TOKEN } from "./bridge";
import { VoiceRequestError } from "./voice";

export interface TranscribeResult {
  text: string;
}

/** Ceiling on a transcription request, so a stalled provider cannot hang a turn. */
const TRANSCRIBE_TIMEOUT_MS = 30_000;

export async function transcribeAudio(
  audioBlob: Blob,
  /** Aborted when the turn is superseded or the user cancels. */
  signal?: AbortSignal,
): Promise<string> {
  const url = `${getServerBaseUrl()}/v1/audio/transcribe`;


  const controller = new AbortController();
  const timer = setTimeout(
    () => controller.abort(new DOMException("timeout", "TimeoutError")),
    TRANSCRIBE_TIMEOUT_MS,
  );
  const onAbort = () => controller.abort(signal?.reason);
  signal?.addEventListener("abort", onAbort, { once: true });

  let response: Response;
  try {
    response = await fetch(url, {
      method: "POST",
      signal: controller.signal,
      headers: {
        Authorization: `Bearer ${DEV_TOKEN}`,
        "Content-Type": audioBlob.type || "audio/webm",
      },
      body: audioBlob,
    });
  } catch {
    if (controller.signal.aborted) {
      const timedOut = (controller.signal.reason as DOMException)?.name === "TimeoutError";
      throw new VoiceRequestError(
        timedOut ? "Transcription timed out." : "Cancelled.",
        timedOut ? "voice_timeout" : "voice_cancelled",
      );
    }
    throw new VoiceRequestError("Couldn't reach the server.", "voice_network_error");
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener("abort", onAbort);
  }

  if (!response.ok) {
    const errorData = await response.json().catch(() => null);
    throw new VoiceRequestError(
      errorData?.message || `Transcription failed with status ${response.status}`,
      errorData?.code ?? (response.status === 401 ? "unauthorized" : "voice_provider_error"),
      response.status,
    );
  }

  const data = (await response.json()) as TranscribeResult;
  return data.text;
}

/**
 * Local device fallback speech recognition using standard browser / WebKit Web Speech API.
 * Operates 100% locally on device without needing external server credentials.
 */
export async function transcribeAudioLocal(signal?: AbortSignal): Promise<string> {
  return new Promise((resolve, reject) => {
    if (typeof window === "undefined") {
      return reject(new VoiceRequestError("Local speech recognition unavailable", "voice_unconfigured"));
    }

    const SpeechRecognition =
      (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition;

    if (!SpeechRecognition) {
      return reject(
        new VoiceRequestError("Device does not support local SpeechRecognition", "voice_unconfigured"),
      );
    }

    const recognition = new SpeechRecognition();
    recognition.continuous = false;
    recognition.interimResults = false;
    recognition.lang = "en-US";

    let resolved = false;

    const cleanup = () => {
      signal?.removeEventListener("abort", onAbort);
      try {
        recognition.stop();
      } catch {}
    };

    const onAbort = () => {
      if (!resolved) {
        resolved = true;
        cleanup();
        reject(new VoiceRequestError("Cancelled.", "voice_cancelled"));
      }
    };

    signal?.addEventListener("abort", onAbort, { once: true });

    recognition.onresult = (event: any) => {
      if (resolved) return;
      resolved = true;
      cleanup();
      const text = event.results?.[0]?.[0]?.transcript || "";
      resolve(text);
    };

    recognition.onerror = (event: any) => {
      if (resolved) return;
      resolved = true;
      cleanup();
      reject(new VoiceRequestError(`Local speech error: ${event.error}`, "voice_provider_error"));
    };

    recognition.onend = () => {
      if (!resolved) {
        resolved = true;
        cleanup();
        resolve("");
      }
    };

    try {
      recognition.start();
    } catch (err: any) {
      resolved = true;
      cleanup();
      reject(
        new VoiceRequestError(
          err?.message || "Failed to start local speech recognition",
          "voice_provider_error",
        ),
      );
    }
  });
}

