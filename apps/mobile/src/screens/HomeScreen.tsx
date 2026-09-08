/**
 * HomeScreen — SiriWave GLSL Shader Voice Interface.
 *
 * Implements:
 * - True Hold-to-Talk (Hold finger down to talk, release finger to start thinking)
 * - Live voice sync with Web Audio API microphone volume
 * - Progressive thinking status with live elapsed seconds (e.g. "Transcribing speech…", "Analyzing prompt…")
 * - Butter-smooth dual-canvas crossfade shader transitions
 */

import { useState, useEffect, useRef, useCallback } from "react";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import MoreHorizRoundedIcon from "@mui/icons-material/MoreHorizRounded";
import type { OrbState } from "orb-ui";

import { loadConnection, type ConnectionState } from "../api/bridge";
import MoreSheet from "../components/MoreSheet";
import { SiriWave, type SiriWaveVariant } from "@/components/ui/siri-wave";
import { useVoiceInput } from "../hooks/useVoiceInput";

const CHECKING: ConnectionState = {
  kind: "checking",
  healthy: false,
  detail: "Checking…",
};

import ButtonBase from "@mui/material/ButtonBase";

import { useVoiceTurn } from "../lib/useVoiceTurn";

// Progressive status stages for AI response synthesis
const AI_THINKING_STAGES = [
  { threshold: 1.2, text: "Transcribing speech…" },
  { threshold: 2.5, text: "Analyzing prompt…" },
  { threshold: 4.0, text: "Reasoning…" },
  { threshold: Infinity, text: "Synthesizing answer…" },
];

import ChecklistRoundedIcon from "@mui/icons-material/ChecklistRounded";
import type { ScreenType } from "../App";

export default function HomeScreen({
  onOpenScreen,
}: {
  onOpenScreen?: (screen: ScreenType) => void;
}) {
  const [connection, setConnection] = useState<ConnectionState>(CHECKING);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [activeVoiceState, setActiveVoiceState] = useState<OrbState | null>(null);
  const [isHolding, setIsHolding] = useState(false);
  const [elapsedMs, setElapsedMs] = useState(0);

  const isHoldingRef = useRef(false);
  const isHandsFreeRef = useRef(false);
  const pointerDownTimeRef = useRef(0);
  const pointerStateAtDownRef = useRef<OrbState | null>(null);
  const voiceTurn = useVoiceTurn();

  const refreshConnection = useCallback(() => {
    setConnection(CHECKING);
    void loadConnection().then((next) => {
      setConnection(next);
    });
  }, []);

  useEffect(() => {
    let cancelled = false;
    const check = async () => {
      const next = await loadConnection();
      if (!cancelled) setConnection(next);
    };

    void check();
    // Poll more frequently (every 5s) when offline to quickly catch server wake-up from Render cold start
    const intervalMs = connection.kind === "connected" ? 15000 : 5000;
    const interval = setInterval(() => {
      void check();
    }, intervalMs);

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [connection.kind]);

  // The controller is authoritative while a turn is in flight. `activeVoiceState`
  // only covers what it does not own: holding to talk, and the initial connect.
  const turnOrbState: OrbState | null =
    voiceTurn.state === "transcribing" || voiceTurn.state === "thinking"
      ? "thinking"
      : voiceTurn.state === "speaking"
        ? "speaking"
        : voiceTurn.state === "error"
          ? "error"
          : null;

  const currentOrbState: OrbState =
    turnOrbState ??
    activeVoiceState ??
    (connection.kind === "checking" ? "connecting" : "idle");

  // Real-time microphone audio capture and OpenAI transcription recorder
  const { audioLevel, stopListening, micError } = useVoiceInput(
    currentOrbState === "listening",
  );

  // Track elapsed timer during active turns (listening, thinking, speaking)
  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (!cancelled) setElapsedMs(0);
    });

    if (currentOrbState === "idle" || currentOrbState === "connecting" || currentOrbState === "error") {
      return;
    }

    const start = Date.now();
    const interval = setInterval(() => {
      setElapsedMs(Date.now() - start);
    }, 100);

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [currentOrbState]);

  // Safety timeout for listening state
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;

    if (currentOrbState === "listening") {
      timer = setTimeout(() => {
        isHoldingRef.current = false;
        isHandsFreeRef.current = false;
        setIsHolding(false);
        setActiveVoiceState("thinking");
      }, isHolding ? 45000 : 8000);
    }

    return () => {
      if (timer) clearTimeout(timer);
    };
  }, [currentOrbState, isHolding]);

  const handlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // ignore
    }

    pointerDownTimeRef.current = Date.now();
    pointerStateAtDownRef.current = currentOrbState;

    if (currentOrbState === "thinking" || currentOrbState === "speaking") {
      // Barge-in: stop the audio AND abandon the turn, so a late transcript or
      // reply cannot arrive later and talk over the next question.
      voiceTurn.interrupt();
      setActiveVoiceState("idle");
      isHoldingRef.current = false;
      isHandsFreeRef.current = false;
      setIsHolding(false);
      return;
    }

    if (currentOrbState === "listening" && isHandsFreeRef.current) {
      return;
    }

    if (currentOrbState === "idle" || currentOrbState === "connecting") {
      voiceTurn.interrupt();
      isHoldingRef.current = true;
      isHandsFreeRef.current = false;
      setIsHolding(true);
      setActiveVoiceState("listening");
    }
  };

  const handlePointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    try {
      if (e.currentTarget.hasPointerCapture(e.pointerId)) {
        e.currentTarget.releasePointerCapture(e.pointerId);
      }
    } catch {
      // ignore
    }

    const duration = Date.now() - pointerDownTimeRef.current;
    const stateAtDown = pointerStateAtDownRef.current;

    if (stateAtDown === "thinking" || stateAtDown === "speaking") {
      return;
    }

    const finishListeningAndSend = async () => {
      isHoldingRef.current = false;
      isHandsFreeRef.current = false;
      setIsHolding(false);

      // The controller owns the turn from here: it mints a turn id, aborts any
      // previous turn, and refuses to apply a result that arrives after the
      // user has moved on. Every failure it reports is shown rather than being
      // logged to a console nobody is reading.
      const audioBlob = await stopListening();
      await voiceTurn.run(audioBlob);
    };

    if (stateAtDown === "listening" && isHandsFreeRef.current) {
      void finishListeningAndSend();
      return;
    }

    if (stateAtDown === "idle" || stateAtDown === "connecting") {
      if (duration >= 350) {
        void finishListeningAndSend();
      } else {
        isHoldingRef.current = false;
        isHandsFreeRef.current = true;
        setIsHolding(false);
      }
    }
  };

  const handlePointerCancel = (e: React.PointerEvent<HTMLDivElement>) => {
    try {
      if (e.currentTarget.hasPointerCapture(e.pointerId)) {
        e.currentTarget.releasePointerCapture(e.pointerId);
      }
    } catch {
      // ignore
    }

    const duration = Date.now() - pointerDownTimeRef.current;
    if (isHoldingRef.current && duration >= 350) {
      isHoldingRef.current = false;
      isHandsFreeRef.current = false;
      setIsHolding(false);
      setActiveVoiceState("thinking");
      return;
    }

    isHoldingRef.current = false;
    isHandsFreeRef.current = false;
    setIsHolding(false);
    setActiveVoiceState("idle");
  };

  const dotColor =
    connection.kind === "checking"
      ? "text.disabled"
      : connection.kind === "offline"
        ? "#7c3aed"
        : connection.healthy
          ? "success.main"
          : "warning.main";

  const elapsedSec = (elapsedMs / 1000).toFixed(1);

  const renderStatus = () => {
    // A microphone we cannot open outranks everything else: without it none of
    // the states below mean anything. This used to be swallowed entirely while
    // the orb animated a fake waveform.
    if (micError) {
      return (
        <Typography
          variant="body2"
          sx={{ fontWeight: 600, color: "warning.main", textAlign: "center", px: 3 }}
        >
          {micError}
        </Typography>
      );
    }

    // Every voice failure now says what actually went wrong. These were
    // console.warn only, so a failed turn looked identical to a turn the
    // assistant simply chose not to answer.
    if (voiceTurn.error) {
      return (
        <Box sx={{ display: "flex", alignItems: "center", gap: 1.5, px: 3 }}>
          <Typography
            variant="body2"
            sx={{ fontWeight: 600, color: "error.main", textAlign: "center" }}
          >
            {voiceTurn.error.message}
          </Typography>
          <ButtonBase
            onClick={voiceTurn.clearError}
            aria-label="Dismiss voice error"
            sx={{ px: 1.5, py: 0.5, borderRadius: 1, fontWeight: 600, color: "text.secondary" }}
          >
            Dismiss
          </ButtonBase>
        </Box>
      );
    }

    if (currentOrbState === "connecting") {
      return (
        <Typography variant="body1" color="text.secondary" sx={{ fontWeight: 500 }}>
          Connecting to server…
        </Typography>
      );
    }

    if (currentOrbState === "listening") {
      return (
        <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
          <Typography
            variant="body1"
            sx={{
              fontWeight: 600,
              color: "info.main",
              animation: "fadeIn 0.25s ease-out",
            }}
          >
            {isHolding ? "Release to answer" : "Listening… (tap to send)"}
          </Typography>
          <Typography
            variant="body2"
            sx={{
              fontFamily: "monospace",
              color: "text.secondary",
              fontWeight: 500,
              fontSize: "0.85rem",
              opacity: 0.85,
            }}
          >
            ({elapsedSec}s)
          </Typography>
        </Box>
      );
    }

    if (currentOrbState === "thinking") {
      const currentSec = elapsedMs / 1000;
      const stage =
        AI_THINKING_STAGES.find((s) => currentSec <= s.threshold) ??
        AI_THINKING_STAGES[AI_THINKING_STAGES.length - 1];

      const displayText =
        voiceTurn.transcript && currentSec > 1.2
          ? `"${voiceTurn.transcript}"`
          : stage.text;

      return (
        <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
          <Typography
            key={displayText}
            variant="body1"
            sx={{
              fontWeight: 600,
              color: "primary.main",
              animation: "fadeIn 0.3s ease-out",
            }}
          >
            {displayText}
          </Typography>
          <Typography
            variant="body2"
            sx={{
              fontFamily: "monospace",
              color: "text.secondary",
              fontWeight: 500,
              fontSize: "0.85rem",
              opacity: 0.85,
            }}
          >
            ({elapsedSec}s)
          </Typography>
        </Box>
      );
    }

    if (currentOrbState === "speaking") {
      return (
        <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
          <Typography
            variant="body1"
            sx={{
              fontWeight: 600,
              color: "success.main",
              animation: "fadeIn 0.25s ease-out",
            }}
          >
            Speaking…
          </Typography>
          <Typography
            variant="body2"
            sx={{
              fontFamily: "monospace",
              color: "text.secondary",
              fontWeight: 500,
              fontSize: "0.85rem",
              opacity: 0.85,
            }}
          >
            ({elapsedSec}s)
          </Typography>
        </Box>
      );
    }

    return (
      <Typography
        variant="body1"
        color="text.secondary"
        sx={{ fontWeight: 500, fontSize: "0.95rem", letterSpacing: "0.01em" }}
      >
        {connection.kind === "offline"
          ? "Hold or Tap to talk • Demo Mode"
          : "Hold or Tap to talk"}
      </Typography>
    );
  };

  const waveVariant: SiriWaveVariant =
    currentOrbState === "thinking" ? "fluid-dots" : "wave";

  const effectiveAudioLevel =
    currentOrbState === "listening"
      ? audioLevel
      : currentOrbState === "speaking"
        ? 0.35 + 0.15 * Math.sin(elapsedMs / 450) * Math.cos(elapsedMs / 700)
        : 0;


  return (
    <Box
      sx={{
        height: "100%",
        width: "100%",
        maxWidth: 500,
        mx: "auto",
        display: "grid",
        gridTemplateRows: "auto 1fr auto",
        justifyItems: "center",
        WebkitTapHighlightColor: "transparent !important",
        userSelect: "none",
        WebkitUserSelect: "none",
        px: 2,
        boxSizing: "border-box",
      }}
    >
      {/* Header */}
      <Box
        sx={{
          width: "100%",
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          pt: 1.5,
          px: 0.5,
        }}
      >
        <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
          <Box
            component="img"
            src="/logo.png"
            alt="Assistant Logo"
            sx={{ width: 28, height: 28, borderRadius: "50%", objectFit: "cover" }}
          />
          <Typography variant="h6" color="text.primary" sx={{ fontSize: "1rem", fontWeight: 700 }}>
            Assistant
          </Typography>
        </Box>
        <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
          {onOpenScreen && (
            <IconButton
              size="small"
              onClick={() => onOpenScreen("tasks")}
              sx={{ color: "primary.main" }}
              aria-label="Open Tasks"
            >
              <ChecklistRoundedIcon />
            </IconButton>
          )}
          <Tooltip title={connection.kind === "offline" ? "Demo Mode (Offline Preview) - Tap to retry" : connection.detail}>
            <Box onClick={refreshConnection} sx={{ display: "flex", alignItems: "center", gap: 0.75, cursor: "pointer" }}>
              {connection.kind === "offline" && (
                <Typography
                  variant="caption"
                  sx={{
                    fontSize: "0.68rem",
                    fontWeight: 600,
                    bgcolor: "rgba(124, 58, 237, 0.1)",
                    color: "#7c3aed",
                    px: 1,
                    py: 0.25,
                    borderRadius: "12px",
                    letterSpacing: "0.02em",
                  }}
                >
                  Demo Mode
                </Typography>
              )}
              <Box
                aria-label={`Server ${connection.kind}`}
                sx={{ width: 8, height: 8, borderRadius: "50%", bgcolor: dotColor }}
              />
            </Box>
          </Tooltip>
        </Box>
      </Box>

      {/* Main Siri Wave GLSL Canvas Container */}
      <Box
        sx={{
          alignSelf: "center",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          gap: 3.5,
          width: "100%",
        }}
      >
        <Box
          onPointerDown={handlePointerDown}
          onPointerUp={handlePointerUp}
          onPointerCancel={handlePointerCancel}
          onContextMenu={(e) => e.preventDefault()}
          sx={{
            cursor: "pointer",
            width: 320,
            height: 320,
            borderRadius: "50%",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            WebkitTapHighlightColor: "transparent !important",
            outline: "none !important",
            touchAction: "none",
            userSelect: "none",
            WebkitUserSelect: "none",
            WebkitTouchCallout: "none",
          }}
        >
          <Box
            sx={{
              pointerEvents: "none",
              width: "100%",
              height: "100%",
              borderRadius: "50%",
              boxShadow: isHolding
                ? "0 25px 75px rgba(124, 58, 237, 0.45)"
                : "0 20px 60px rgba(0,0,0,0.25)",
              transform: isHolding ? "scale(0.96)" : "scale(1.0)",
              transition: "transform 0.18s cubic-bezier(0.16, 1, 0.3, 1), box-shadow 0.18s ease-out",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <SiriWave
              variant={waveVariant}
              size={320}
              renderScale={1.0}
              audioLevel={effectiveAudioLevel}
            />
          </Box>
        </Box>

        <Box
          sx={{
            textAlign: "center",
            minHeight: 48,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          {renderStatus()}
        </Box>
      </Box>

      {/* Footer drawer button */}
      <IconButton
        aria-label="Everything else"
        onClick={() => setSheetOpen(true)}
        sx={{
          mb: 3,
          p: 2,
          position: "relative",
          zIndex: 10,
          color: "text.secondary",
          WebkitTapHighlightColor: "transparent !important",
          outline: "none !important",
        }}
      >
        <MoreHorizRoundedIcon sx={{ fontSize: 32 }} />
      </IconButton>

      <MoreSheet
        open={sheetOpen}
        onClose={() => setSheetOpen(false)}
        connection={connection}
        onSelectScreen={(screen) => {
          if (onOpenScreen) onOpenScreen(screen);
        }}
      />
    </Box>
  );
}

