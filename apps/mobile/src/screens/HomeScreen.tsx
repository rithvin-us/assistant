/**
 * HomeScreen — SiriWave GLSL Shader Voice Interface.
 *
 * Implements:
 * - True Hold-to-Talk (Hold finger down to talk, release finger to start thinking)
 * - Live voice sync with Web Audio API microphone volume
 * - Progressive thinking status with live elapsed seconds (e.g. "Transcribing speech…", "Analyzing prompt…")
 * - Butter-smooth dual-canvas crossfade shader transitions
 */

import { useState, useEffect, useRef } from "react";
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

import { transcribeAudio } from "../api/transcribe";

// Progressive status stages for AI response synthesis
const AI_THINKING_STAGES = [
  { threshold: 1.2, text: "Transcribing speech…" },
  { threshold: 2.5, text: "Analyzing prompt…" },
  { threshold: 4.0, text: "Reasoning…" },
  { threshold: Infinity, text: "Synthesizing answer…" },
];

import ChecklistRoundedIcon from "@mui/icons-material/ChecklistRounded";
import type { ProductivityTab } from "./ProductivityScreen";

export default function HomeScreen({
  onOpenProductivity,
}: {
  onOpenProductivity?: (tab: ProductivityTab) => void;
}) {
  const [connection, setConnection] = useState<ConnectionState>(CHECKING);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [activeVoiceState, setActiveVoiceState] = useState<OrbState | null>(null);
  const [isHolding, setIsHolding] = useState(false);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [transcribedText, setTranscribedText] = useState<string | null>(null);

  const isHoldingRef = useRef(false);
  const isHandsFreeRef = useRef(false);
  const pointerDownTimeRef = useRef(0);
  const pointerStateAtDownRef = useRef<OrbState | null>(null);

  useEffect(() => {
    let cancelled = false;
    void loadConnection().then((next) => {
      if (!cancelled) setConnection(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const currentOrbState: OrbState =
    activeVoiceState ??
    (connection.kind === "checking"
      ? "connecting"
      : "idle");

  // Real-time microphone audio capture and OpenAI transcription recorder
  const { audioLevel, stopListening } = useVoiceInput(currentOrbState === "listening");

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

  // Automated state progression for thinking and speaking
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;

    if (currentOrbState === "listening") {
      timer = setTimeout(() => {
        isHoldingRef.current = false;
        isHandsFreeRef.current = false;
        setIsHolding(false);
        setActiveVoiceState("thinking");
      }, isHolding ? 45000 : 5500);
    } else if (currentOrbState === "thinking") {
      timer = setTimeout(() => {
        setActiveVoiceState("speaking");
      }, 4200);
    } else if (currentOrbState === "speaking") {
      timer = setTimeout(() => {
        setActiveVoiceState("idle");
        setTranscribedText(null);
      }, 5000);
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
      setActiveVoiceState("thinking");

      try {
        const audioBlob = await stopListening();
        if (audioBlob && audioBlob.size > 200 && connection.healthy) {
          const text = await transcribeAudio(audioBlob);
          if (text) {
            setTranscribedText(text);
            console.log("Transcribed via OpenAI:", text);
          }
        }
      } catch (err) {
        console.warn("Transcription notice:", err);
      }
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
        transcribedText && currentSec > 1.2
          ? `"${transcribedText}"`
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
        display: "grid",
        gridTemplateRows: "auto 1fr auto",
        justifyItems: "center",
        WebkitTapHighlightColor: "transparent !important",
        userSelect: "none",
        WebkitUserSelect: "none",
        px: 2,
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
          {onOpenProductivity && (
            <IconButton
              size="small"
              onClick={() => onOpenProductivity("tasks")}
              sx={{ color: "primary.main" }}
              aria-label="Open Tasks"
            >
              <ChecklistRoundedIcon />
            </IconButton>
          )}
          <Tooltip title={connection.kind === "offline" ? "Demo Mode (Offline Preview)" : connection.detail}>
            <Box sx={{ display: "flex", alignItems: "center", gap: 0.75 }}>
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
          color: "text.secondary",
          WebkitTapHighlightColor: "transparent !important",
          outline: "none !important",
        }}
      >
        <MoreHorizRoundedIcon />
      </IconButton>

      <MoreSheet
        open={sheetOpen}
        onClose={() => setSheetOpen(false)}
        connection={connection}
        onSelectTab={(tab) => {
          if (onOpenProductivity) onOpenProductivity(tab);
        }}
      />
    </Box>
  );
}

