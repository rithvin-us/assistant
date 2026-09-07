/**
 * Academic Overview.
 *
 * A short, honest summary: how many assignments are due in the next seven
 * days, how many are overdue, what is coming next, and what was announced
 * recently. Every number is counted from rows on the server — none of it is
 * inferred, and no model is involved.
 *
 * Deliberately not a dashboard. Four numbers and two short lists.
 */

import { useCallback, useEffect, useState } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import IconButton from "@mui/material/IconButton";
import Divider from "@mui/material/Divider";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemText from "@mui/material/ListItemText";
import Alert from "@mui/material/Alert";
import CircularProgress from "@mui/material/CircularProgress";
import Chip from "@mui/material/Chip";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";

import PullToRefresh from "../components/PullToRefresh";

import { fetchAcademicOverview, formatDue, formatSynced } from "../api/academic";
import type { AcademicOverview } from "../api/types";

interface Props {
  onBack: () => void;
  onOpenClassroom?: () => void;
}

function Stat({ value, label }: { value: number; label: string }) {
  return (
    <Box sx={{ flex: 1, textAlign: "center" }}>
      <Typography variant="h4" sx={{ fontWeight: 700, lineHeight: 1.1 }}>
        {value}
      </Typography>
      <Typography variant="caption" sx={{ color: "text.secondary" }}>
        {label}
      </Typography>
    </Box>
  );
}

export default function AcademicScreen({ onBack, onOpenClassroom }: Props) {
  const [overview, setOverview] = useState<AcademicOverview | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Every state write happens after an await, so nothing is set synchronously
  // while the effect body runs.
  const load = useCallback(async (cancelled?: () => boolean) => {
    try {
      const next = await fetchAcademicOverview();
      if (cancelled?.()) return;
      setOverview(next);
      setError(null);
    } catch (e: unknown) {
      if (cancelled?.()) return;
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (!cancelled?.()) setBusy(false);
    }
  }, []);

  const handleRefresh = async () => {
    await load();
  };

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      if (cancelled) return;
      await load(() => cancelled);
    })();
    return () => {
      cancelled = true;
    };
  }, [load]);

  return (
    <Box sx={{ height: "100%", display: "flex", flexDirection: "column" }}>
      <Box sx={{ display: "flex", alignItems: "center", gap: 1, px: 1, pt: 1 }}>
        <IconButton onClick={onBack} aria-label="Back">
          <ArrowBackRoundedIcon />
        </IconButton>
        <Typography variant="h6" sx={{ flex: 1, fontWeight: 700 }}>
          Academic
        </Typography>
      </Box>

      {error && (
        <Alert severity="error" sx={{ mx: 2, mb: 1 }} onClose={() => setError(null)}>
          {error}
        </Alert>
      )}

      {busy && (
        <Box sx={{ display: "flex", justifyContent: "center", py: 4 }}>
          <CircularProgress size={22} />
        </Box>
      )}

      {overview && (
        <PullToRefresh onRefresh={handleRefresh}>
          <Box sx={{ display: "flex", px: 2, py: 2.5 }}>
            <Stat value={overview.due_this_week} label="due this week" />
            <Stat value={overview.overdue} label="overdue" />
            <Stat value={overview.course_count} label="courses" />
          </Box>

          {/* Says how old the data is rather than implying it is live. */}
          <Typography
            variant="caption"
            sx={{ px: 2, pb: 1.5, color: "text.secondary", display: "block" }}
          >
            {formatSynced(overview.oldest_synced_at)}
          </Typography>

          <Divider />

          <Typography variant="overline" sx={{ px: 2, pt: 2, display: "block", color: "text.secondary" }}>
            Next up
          </Typography>

          {overview.upcoming.length === 0 && (
            <Typography variant="body2" sx={{ px: 2, py: 2, color: "text.secondary" }}>
              Nothing outstanding.{" "}
              {overview.course_count === 0 && onOpenClassroom && (
                <Box
                  component="span"
                  onClick={onOpenClassroom}
                  sx={{ color: "primary.main", cursor: "pointer" }}
                >
                  Sync Classroom to import coursework.
                </Box>
              )}
            </Typography>
          )}

          {overview.upcoming.map((d) => (
            <Box key={`${d.source}-${d.external_id ?? d.task_id ?? d.title}`}>
              <ListItemButton
                component={d.alternate_link ? "a" : "div"}
                href={d.alternate_link ?? undefined}
                target={d.alternate_link ? "_blank" : undefined}
                rel={d.alternate_link ? "noreferrer" : undefined}
              >
                <ListItemText
                  primary={d.title}
                  secondary={[d.context, formatDue(d.due_at)].filter(Boolean).join(" — ")}
                  slotProps={{
                    primary: { sx: { fontWeight: 600 } },
                    secondary: {
                      color: d.is_overdue ? "error.main" : "text.secondary",
                    },
                  }}
                />
                {/* Imported work is labelled so it is distinguishable from a
                    task the user typed themselves. */}
                {d.source === "google_classroom" && (
                  <Chip label="Classroom" size="small" variant="outlined" />
                )}
              </ListItemButton>
              <Divider component="li" sx={{ listStyle: "none" }} />
            </Box>
          ))}

          {overview.recent_announcements.length > 0 && (
            <>
              <Typography
                variant="overline"
                sx={{ px: 2, pt: 3, display: "block", color: "text.secondary" }}
              >
                Recent announcements
              </Typography>
              {overview.recent_announcements.map((a) => (
                <Box key={a.external_id}>
                  <ListItemButton
                    component="a"
                    href={a.alternate_link ?? undefined}
                    target="_blank"
                    rel="noreferrer"
                  >
                    <ListItemText
                      primary={a.text || "(no text)"}
                      slotProps={{
                        primary: {
                          variant: "body2",
                          sx: {
                            display: "-webkit-box",
                            WebkitLineClamp: 2,
                            WebkitBoxOrient: "vertical",
                            overflow: "hidden",
                          },
                        },
                      }}
                    />
                  </ListItemButton>
                  <Divider component="li" sx={{ listStyle: "none" }} />
                </Box>
              ))}
            </>
          )}
        </PullToRefresh>
      )}
    </Box>
  );
}
