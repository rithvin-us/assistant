import { useEffect, useRef, useState, useCallback } from "react";

/** Why capture is unavailable, when it is. */
export type MicrophoneStatus =
  | "ok"
  /** The user refused, or Android has not granted the runtime permission. */
  | "denied"
  /** No input device, or another app holds it. */
  | "unavailable"
  /** The WebView exposes no capture API at all. */
  | "unsupported";

export interface UseVoiceInputResult {
  audioLevel: number; // 0.0 to 1.0
  isListening: boolean;
  startListening: () => Promise<void>;
  stopListening: () => Promise<Blob | null>;
  /** `ok` until a capture attempt actually fails. */
  micStatus: MicrophoneStatus;
  /** Human-readable reason, safe to show. Null when capture is fine. */
  micError: string | null;
}

export function useVoiceInput(active: boolean): UseVoiceInputResult {
  const [audioLevel, setAudioLevel] = useState<number>(0);
  const audioContextRef = useRef<AudioContext | null>(null);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const sourceRef = useRef<MediaStreamAudioSourceNode | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const audioChunksRef = useRef<Blob[]>([]);
  const rafRef = useRef<number>(0);
  const smoothedLevelRef = useRef<number>(0);
  const [micStatus, setMicStatus] = useState<MicrophoneStatus>("ok");
  const [micError, setMicError] = useState<string | null>(null);

  const stopListening = useCallback(async (): Promise<Blob | null> => {
    if (rafRef.current) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = 0;
    }

    let recordedBlob: Blob | null = null;
    const recorder = mediaRecorderRef.current;

    if (recorder && recorder.state !== "inactive") {
      try {
        const stopPromise = new Promise<void>((resolve) => {
          recorder.onstop = () => resolve();
        });
        recorder.stop();
        await stopPromise;
      } catch {
        // ignore recorder error
      }
    }

    if (audioChunksRef.current.length > 0) {
      const mimeType = recorder?.mimeType || "audio/webm";
      recordedBlob = new Blob(audioChunksRef.current, { type: mimeType });
      audioChunksRef.current = [];
    }

    mediaRecorderRef.current = null;

    if (streamRef.current) {
      streamRef.current.getTracks().forEach((t) => t.stop());
      streamRef.current = null;
    }

    if (audioContextRef.current && audioContextRef.current.state !== "closed") {
      void audioContextRef.current.close().catch(() => {});
      audioContextRef.current = null;
    }

    analyserRef.current = null;
    sourceRef.current = null;
    smoothedLevelRef.current = 0;
    setAudioLevel(0);

    return recordedBlob;
  }, []);

  const startListening = useCallback(async () => {
    await stopListening();

    try {
      if (typeof navigator !== "undefined" && navigator.mediaDevices?.getUserMedia) {
        const stream = await navigator.mediaDevices.getUserMedia({
          audio: {
            echoCancellation: true,
            noiseSuppression: true,
            autoGainControl: true,
          },
        });
        streamRef.current = stream;
        setMicStatus("ok");
        setMicError(null);

        // Setup MediaRecorder for OpenAI voice transcription
        try {
          audioChunksRef.current = [];
          const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
            ? "audio/webm;codecs=opus"
            : MediaRecorder.isTypeSupported("audio/webm")
              ? "audio/webm"
              : MediaRecorder.isTypeSupported("audio/mp4")
                ? "audio/mp4"
                : "";

          const recorder = mimeType
            ? new MediaRecorder(stream, { mimeType })
            : new MediaRecorder(stream);

          recorder.ondataavailable = (event) => {
            if (event.data && event.data.size > 0) {
              audioChunksRef.current.push(event.data);
            }
          };
          recorder.start(100);
          mediaRecorderRef.current = recorder;
        } catch (recErr) {
          console.warn("MediaRecorder init failed, fallback audio level only:", recErr);
        }

        // Setup Web Audio Analyser for SiriWave real-time animation
        const AudioCtx =
          window.AudioContext ||
          (window as unknown as { webkitAudioContext: typeof AudioContext })
            .webkitAudioContext;
        const ctx = new AudioCtx();
        audioContextRef.current = ctx;

        const analyser = ctx.createAnalyser();
        analyser.fftSize = 256;
        analyser.smoothingTimeConstant = 0.8;
        analyserRef.current = analyser;

        const source = ctx.createMediaStreamSource(stream);
        source.connect(analyser);
        sourceRef.current = source;

        const dataArray = new Uint8Array(analyser.frequencyBinCount);

        const update = () => {
          if (!analyserRef.current) return;
          analyserRef.current.getByteFrequencyData(dataArray);

          let sum = 0;
          for (let i = 0; i < dataArray.length; i++) {
            sum += dataArray[i];
          }
          const avg = sum / dataArray.length;
          const rawLevel = Math.min(Math.max((avg - 12) / 65, 0), 1.0);

          smoothedLevelRef.current =
            smoothedLevelRef.current * 0.84 + rawLevel * 0.16;

          setAudioLevel(smoothedLevelRef.current);
          rafRef.current = requestAnimationFrame(update);
        };
        update();
        return;
      }
    } catch (err) {
      // A denied microphone used to fall through to the synthetic waveform
      // below, so the orb animated as though it were hearing the user. That is
      // the app lying about whether it is listening. Report it instead, and do
      // not animate.
      const name = (err as DOMException)?.name;
      if (name === "NotAllowedError" || name === "SecurityError") {
        setMicStatus("denied");
        setMicError(
          "Microphone access is off. Turn it on for Assistant in Android settings.",
        );
      } else if (name === "NotFoundError" || name === "NotReadableError") {
        setMicStatus("unavailable");
        setMicError("No microphone is available, or another app is using it.");
      } else {
        setMicStatus("unavailable");
        setMicError("Couldn't start the microphone.");
      }
      setAudioLevel(0);
      return;
    }

    // Reached only when the WebView exposes no capture API. The animation below
    // is decorative and explicitly not a recording.
    setMicStatus("unsupported");
    setMicError("This device can't capture audio in the app.");

    let t = 0;
    const fallbackUpdate = () => {
      t += 0.02;
      const simulated = Math.max(
        0,
        0.35 + 0.25 * Math.sin(t * 1.6) * Math.cos(t * 0.9),
      );
      smoothedLevelRef.current =
        smoothedLevelRef.current * 0.88 + simulated * 0.12;
      setAudioLevel(smoothedLevelRef.current);
      rafRef.current = requestAnimationFrame(fallbackUpdate);
    };
    fallbackUpdate();
  }, [stopListening]);

  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) return;
      if (active) {
        void startListening();
      } else {
        void stopListening();
      }
    });
    return () => {
      cancelled = true;
      void stopListening();
    };
  }, [active, startListening, stopListening]);


  return {
    audioLevel,
    isListening: active,
    startListening,
    stopListening,
    micStatus,
    micError,
  };
}
