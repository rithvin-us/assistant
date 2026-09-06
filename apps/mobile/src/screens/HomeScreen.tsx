/**
 * Home.
 *
 * At this milestone Home's only real job is to answer one question honestly:
 * can this device reach the assistant server, and is that server healthy? The
 * attention feed, deadlines, free-time and quick capture take this slot later.
 */

import { useEffect, useState } from "react";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import CircularProgress from "@mui/material/CircularProgress";
import Paper from "@mui/material/Paper";
import Stack from "@mui/material/Stack";
import Typography from "@mui/material/Typography";
import MicRoundedIcon from "@mui/icons-material/MicRounded";

import { SERVER_BASE_URL, localCacheReady, probeServer, type ProbeResult } from "../api/bridge";
import { PROTOCOL_VERSION } from "../api/types";

type Probe = { status: "loading" } | { status: "done"; result: ProbeResult };

/** Gathers everything Home shows. Pure data in, no React state touched. */
async function loadStatus(): Promise<{ result: ProbeResult; cacheReady: boolean }> {
  const [result, cacheReady] = await Promise.all([probeServer(), localCacheReady()]);
  return { result, cacheReady };
}

export default function HomeScreen() {
  const [probe, setProbe] = useState<Probe>({ status: "loading" });
  const [cacheReady, setCacheReady] = useState<boolean | null>(null);

  // The probe is an external system, so it is synchronised from an effect. State
  // is only written from the promise callback, never synchronously in the effect
  // body, and the cancellation flag stops a late response from writing to an
  // unmounted component.
  useEffect(() => {
    let cancelled = false;

    void loadStatus().then(({ result, cacheReady: ready }) => {
      if (cancelled) return;
      setProbe({ status: "done", result });
      setCacheReady(ready);
    });

    return () => {
      cancelled = true;
    };
  }, []);

  const recheck = () => {
    setProbe({ status: "loading" });
    setCacheReady(null);
    void loadStatus().then(({ result, cacheReady: ready }) => {
      setProbe({ status: "done", result });
      setCacheReady(ready);
    });
  };

  return (
    <Stack spacing={2.5}>
      <Box>
        <Typography variant="h1">Assistant</Typography>
        <Typography variant="body2" color="text.secondary">
          Milestone 0 — foundation only. No model, no integrations.
        </Typography>
      </Box>

      <Paper sx={{ p: 2 }}>
        <Typography variant="h2" gutterBottom>
          Server
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
          {SERVER_BASE_URL}
        </Typography>

        {probe.status === "loading" ? (
          <Stack direction="row" spacing={1.5} sx={{ alignItems: "center" }}>
            <CircularProgress size={18} />
            <Typography variant="body2">Checking…</Typography>
          </Stack>
        ) : (
          <ServerStatus result={probe.result} />
        )}

        <Button size="small" onClick={recheck} sx={{ mt: 1.5 }}>
          Check again
        </Button>
      </Paper>

      <Paper sx={{ p: 2 }}>
        <Typography variant="h2" gutterBottom>
          Local cache
        </Typography>
        <Typography variant="body2" color="text.secondary">
          {cacheReady === null
            ? "Checking…"
            : cacheReady
              ? "SQLite cache open. Offline capture will use it."
              : "Unavailable. The app runs, but nothing can be captured offline."}
        </Typography>
      </Paper>

      <Button
        variant="contained"
        size="large"
        startIcon={<MicRoundedIcon />}
        disabled
        sx={{ py: 1.5 }}
      >
        Voice — not built yet
      </Button>
    </Stack>
  );
}

function ServerStatus({ result }: { result: ProbeResult }) {
  if (result.state === "unreachable") {
    return (
      <Alert severity="error" variant="outlined">
        Unreachable. {result.reason}
      </Alert>
    );
  }

  const { health, latencyMs } = result;
  // A protocol mismatch means one side is running an older build. Saying so is
  // far more useful than letting a field silently deserialise to undefined.
  const mismatch = health.protocol_version !== PROTOCOL_VERSION;

  return (
    <Stack spacing={1.5}>
      <Stack direction="row" spacing={1} useFlexGap sx={{ flexWrap: "wrap" }}>
        <Chip
          size="small"
          color={health.status === "ok" ? "success" : "warning"}
          label={health.status === "ok" ? "Healthy" : "Degraded"}
        />
        <Chip size="small" variant="outlined" label={`v${health.version}`} />
        <Chip size="small" variant="outlined" label={`${latencyMs} ms`} />
      </Stack>

      {health.status === "degraded" && (
        <Typography variant="body2" color="text.secondary">
          Server is up but has no database. Set DATABASE_URL to enable persistence.
        </Typography>
      )}

      {mismatch && (
        <Alert severity="warning" variant="outlined">
          Protocol mismatch: app expects v{PROTOCOL_VERSION}, server speaks v
          {health.protocol_version}.
        </Alert>
      )}
    </Stack>
  );
}
