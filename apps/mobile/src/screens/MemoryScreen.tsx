/**
 * Long-term Memory screen (M7).
 *
 * A standalone memory manager. Not an AI chat interface: the model does not
 * live here. Memories are durable records the server owns; this screen lets
 * the user browse, search, filter, edit, archive and restore them, and see
 * where each memory came from.
 *
 * Layout matches the productivity screens (Todoist-inspired, Material UI):
 * search bar, chips row for filters, list of cards, floating "add" button, and
 * a modal detail/edit sheet. Everything on the page is real data pulled from
 * `/v1/memories`; nothing is faked.
 */

import { useState, useEffect, useMemo } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import TextField from "@mui/material/TextField";
import InputAdornment from "@mui/material/InputAdornment";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import Card from "@mui/material/Card";
import CardContent from "@mui/material/CardContent";
import IconButton from "@mui/material/IconButton";
import Fab from "@mui/material/Fab";
import Dialog from "@mui/material/Dialog";
import DialogTitle from "@mui/material/DialogTitle";
import DialogContent from "@mui/material/DialogContent";
import DialogActions from "@mui/material/DialogActions";
import MenuItem from "@mui/material/MenuItem";
import Alert from "@mui/material/Alert";
import CircularProgress from "@mui/material/CircularProgress";
import Snackbar from "@mui/material/Snackbar";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import Tooltip from "@mui/material/Tooltip";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import AddRoundedIcon from "@mui/icons-material/AddRounded";
import EditOutlinedIcon from "@mui/icons-material/EditOutlined";
import ArchiveOutlinedIcon from "@mui/icons-material/ArchiveOutlined";
import UnarchiveOutlinedIcon from "@mui/icons-material/UnarchiveOutlined";
import HistoryToggleOffIcon from "@mui/icons-material/HistoryToggleOff";
import CompareArrowsRoundedIcon from "@mui/icons-material/CompareArrowsRounded";
import InsightsRoundedIcon from "@mui/icons-material/InsightsRounded";

import type { MemoryItem, MemoryKind } from "../api/types";
import {
  listMemories,
  createMemory,
  updateMemory,
  archiveMemory,
  restoreMemory,
  MEMORY_KIND_LABELS,
  MEMORY_SOURCE_LABELS,
  importanceLabel,
} from "../api/memory";

interface MemoryScreenProps {
  onBack?: () => void;
}

type LifecycleFilter = "active" | "archived";

const KIND_ORDER: MemoryKind[] = [
  "preference",
  "fact",
  "idea",
  "commitment",
  "project",
  "temporary",
];

const KIND_COLORS: Record<MemoryKind, string> = {
  preference: "#7C3AED",
  fact: "#0F9D58",
  idea: "#F4B400",
  commitment: "#DC4C3E",
  project: "#2563EB",
  temporary: "#6B7280",
};

function relative(iso: string | null | undefined): string {
  if (!iso) return "never";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "unknown";
  const diff = Date.now() - then;
  const minutes = Math.round(diff / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.round(days / 30);
  if (months < 12) return `${months}mo ago`;
  return `${Math.round(months / 12)}y ago`;
}

function formatExact(iso: string | null | undefined): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}

function importanceColor(importance: number): string {
  if (importance >= 5) return "#DC4C3E";
  if (importance >= 4) return "#F4B400";
  if (importance <= 2) return "#9CA3AF";
  return "#2563EB";
}

interface DraftMemory {
  kind: MemoryKind;
  content: string;
  importance: number;
  confidence: number;
  expiresAt: string;
}

const EMPTY_DRAFT: DraftMemory = {
  kind: "preference",
  content: "",
  importance: 3,
  confidence: 1.0,
  expiresAt: "",
};

export default function MemoryScreen({ onBack }: MemoryScreenProps) {
  const [memories, setMemories] = useState<MemoryItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [searchQuery, setSearchQuery] = useState("");
  const [kindFilter, setKindFilter] = useState<MemoryKind | "all">("all");
  const [lifecycleFilter, setLifecycleFilter] =
    useState<LifecycleFilter>("active");
  const [minImportance, setMinImportance] = useState<number>(1);

  const [detail, setDetail] = useState<MemoryItem | null>(null);
  const [editing, setEditing] = useState<MemoryItem | null>(null);
  const [editDraft, setEditDraft] = useState<DraftMemory>(EMPTY_DRAFT);
  const [creating, setCreating] = useState(false);
  const [createDraft, setCreateDraft] = useState<DraftMemory>(EMPTY_DRAFT);

  const reload = async () => {
    setLoading(true);
    setErrorMsg(null);
    try {
      const rows = await listMemories({
        q: searchQuery.trim() || undefined,
        kind: kindFilter === "all" ? undefined : [kindFilter],
        lifecycle: [lifecycleFilter],
        minImportance: minImportance > 1 ? minImportance : undefined,
        limit: 100,
      });
      setMemories(rows);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to load memories";
      setErrorMsg(message);
      setMemories([]);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      setLoading(true);
      try {
        const rows = await listMemories({
          q: searchQuery.trim() || undefined,
          kind: kindFilter === "all" ? undefined : [kindFilter],
          lifecycle: [lifecycleFilter],
          minImportance: minImportance > 1 ? minImportance : undefined,
          limit: 100,
        });
        if (!cancelled) {
          setMemories(rows);
          setErrorMsg(null);
        }
      } catch (err) {
        if (!cancelled) {
          const message =
            err instanceof Error ? err.message : "Failed to load memories";
          setErrorMsg(message);
          setMemories([]);
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [searchQuery, kindFilter, lifecycleFilter, minImportance]);

  const counts = useMemo(() => {
    const byKind = new Map<MemoryKind, number>();
    for (const memory of memories) {
      byKind.set(memory.kind, (byKind.get(memory.kind) ?? 0) + 1);
    }
    return byKind;
  }, [memories]);

  const handleCreate = async () => {
    const content = createDraft.content.trim();
    if (!content) return;
    try {
      await createMemory({
        kind: createDraft.kind,
        content,
        importance: createDraft.importance,
        confidence: createDraft.confidence,
        source_kind: "explicit_user_input",
        expires_at:
          createDraft.kind === "temporary" && createDraft.expiresAt
            ? new Date(createDraft.expiresAt).toISOString()
            : undefined,
      });
      setCreating(false);
      setCreateDraft(EMPTY_DRAFT);
      setNotice("Memory saved.");
      await reload();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Could not save memory";
      setErrorMsg(message);
    }
  };

  const handleEditSave = async () => {
    if (!editing) return;
    try {
      await updateMemory(editing.id, {
        kind: editDraft.kind,
        content: editDraft.content,
        importance: editDraft.importance,
        confidence: editDraft.confidence,
        expires_at:
          editDraft.kind === "temporary"
            ? editDraft.expiresAt
              ? new Date(editDraft.expiresAt).toISOString()
              : undefined
            : null,
      });
      setEditing(null);
      setDetail(null);
      setNotice("Memory updated.");
      await reload();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Update failed";
      setErrorMsg(message);
    }
  };

  const handleArchive = async (memory: MemoryItem) => {
    try {
      await archiveMemory(memory.id);
      setDetail(null);
      setNotice("Memory archived.");
      await reload();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Could not archive";
      setErrorMsg(message);
    }
  };

  const handleRestore = async (memory: MemoryItem) => {
    try {
      await restoreMemory(memory.id);
      setDetail(null);
      setNotice("Memory restored.");
      await reload();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Could not restore";
      setErrorMsg(message);
    }
  };

  const openEdit = (memory: MemoryItem) => {
    setEditing(memory);
    setEditDraft({
      kind: memory.kind,
      content: memory.content,
      importance: memory.importance,
      confidence: memory.confidence,
      expiresAt: memory.expires_at
        ? memory.expires_at.slice(0, 16)
        : "",
    });
  };

  const openCreate = () => {
    setCreateDraft(EMPTY_DRAFT);
    setCreating(true);
  };

  return (
    <Box
      sx={{
        height: "100%",
        display: "flex",
        flexDirection: "column",
        bgcolor: "#FAFAFA",
      }}
    >
      {/* Header */}
      <Box
        sx={{
          px: 2,
          py: 1.25,
          display: "flex",
          alignItems: "center",
          gap: 1.25,
          bgcolor: "#FFFFFF",
          borderBottom: "1px solid #EEE",
        }}
      >
        {onBack && (
          <IconButton onClick={onBack} aria-label="Back" size="small">
            <ArrowBackRoundedIcon />
          </IconButton>
        )}
        <InsightsRoundedIcon sx={{ color: "#7C3AED" }} />
        <Box sx={{ flex: 1 }}>
          <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
            Memory
          </Typography>
          <Typography variant="caption" color="text.secondary">
            {loading
              ? "Loading…"
              : `${memories.length} ${lifecycleFilter} memor${
                  memories.length === 1 ? "y" : "ies"
                }`}
          </Typography>
        </Box>
      </Box>

      {/* Search bar */}
      <Box sx={{ px: 2, pt: 1.5, pb: 0.5 }}>
        <TextField
          fullWidth
          size="small"
          placeholder="Search memory"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <SearchRoundedIcon fontSize="small" />
                </InputAdornment>
              ),
            },
          }}
        />
      </Box>

      {/* Lifecycle toggle */}
      <Stack
        direction="row"
        spacing={1}
        sx={{ px: 2, pt: 1.5, overflowX: "auto" }}
      >
        <Chip
          label="Active"
          color={lifecycleFilter === "active" ? "primary" : "default"}
          variant={lifecycleFilter === "active" ? "filled" : "outlined"}
          onClick={() => setLifecycleFilter("active")}
          size="small"
        />
        <Chip
          label="Archived"
          color={lifecycleFilter === "archived" ? "primary" : "default"}
          variant={lifecycleFilter === "archived" ? "filled" : "outlined"}
          onClick={() => setLifecycleFilter("archived")}
          size="small"
        />
      </Stack>

      {/* Kind chips */}
      <Stack
        direction="row"
        spacing={1}
        sx={{
          px: 2,
          pt: 1,
          overflowX: "auto",
          whiteSpace: "nowrap",
          scrollbarWidth: "none",
        }}
      >
        <Chip
          label={`All${memories.length ? ` (${memories.length})` : ""}`}
          onClick={() => setKindFilter("all")}
          variant={kindFilter === "all" ? "filled" : "outlined"}
          size="small"
        />
        {KIND_ORDER.map((kind) => (
          <Chip
            key={kind}
            label={`${MEMORY_KIND_LABELS[kind]}${
              counts.get(kind) ? ` (${counts.get(kind)})` : ""
            }`}
            onClick={() =>
              setKindFilter((current) => (current === kind ? "all" : kind))
            }
            variant={kindFilter === kind ? "filled" : "outlined"}
            size="small"
            sx={{
              borderColor: KIND_COLORS[kind],
              color: kindFilter === kind ? "#FFF" : KIND_COLORS[kind],
              bgcolor: kindFilter === kind ? KIND_COLORS[kind] : "transparent",
            }}
          />
        ))}
      </Stack>

      {/* Importance slider */}
      <Box sx={{ px: 3, pt: 1.5, pb: 1 }}>
        <Typography variant="caption" color="text.secondary">
          Minimum importance: {importanceLabel(minImportance)}
        </Typography>
        <Slider
          value={minImportance}
          min={1}
          max={5}
          step={1}
          marks
          size="small"
          onChange={(_, value) =>
            setMinImportance(Array.isArray(value) ? value[0] : value)
          }
        />
      </Box>

      {/* Body */}
      <Box sx={{ flex: 1, overflowY: "auto", px: 2, pb: 12 }}>
        {errorMsg && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {errorMsg}
          </Alert>
        )}
        {loading && memories.length === 0 && (
          <Box sx={{ display: "flex", justifyContent: "center", py: 6 }}>
            <CircularProgress size={24} />
          </Box>
        )}
        {!loading && memories.length === 0 && !errorMsg && (
          <Box sx={{ textAlign: "center", py: 6, color: "text.secondary" }}>
            <Typography variant="body2">
              {lifecycleFilter === "active"
                ? "No memories yet. Tap the + button to add one, or say “Remember that …” in a conversation."
                : "Nothing archived."}
            </Typography>
          </Box>
        )}
        <Stack spacing={1.25}>
          {memories.map((memory) => (
            <Card
              key={memory.id}
              variant="outlined"
              onClick={() => setDetail(memory)}
              sx={{
                cursor: "pointer",
                borderColor: "#E5E7EB",
                "&:hover": { borderColor: KIND_COLORS[memory.kind] },
              }}
            >
              <CardContent sx={{ py: 1.5, "&:last-child": { pb: 1.5 } }}>
                <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                  <Chip
                    label={MEMORY_KIND_LABELS[memory.kind]}
                    size="small"
                    sx={{
                      bgcolor: `${KIND_COLORS[memory.kind]}15`,
                      color: KIND_COLORS[memory.kind],
                      fontWeight: 600,
                      height: 20,
                    }}
                  />
                  <Chip
                    label={`Importance ${memory.importance}/5`}
                    size="small"
                    variant="outlined"
                    sx={{
                      height: 20,
                      color: importanceColor(memory.importance),
                      borderColor: importanceColor(memory.importance),
                    }}
                  />
                  {memory.lifecycle === "superseded" && (
                    <Tooltip title="Replaced by a newer memory">
                      <CompareArrowsRoundedIcon fontSize="small" color="warning" />
                    </Tooltip>
                  )}
                  {memory.kind === "temporary" && memory.expires_at && (
                    <Tooltip title={`Expires ${formatExact(memory.expires_at)}`}>
                      <HistoryToggleOffIcon fontSize="small" color="action" />
                    </Tooltip>
                  )}
                </Box>
                <Typography variant="body1" sx={{ mt: 0.75, fontWeight: 500 }}>
                  {memory.content}
                </Typography>
                <Box
                  sx={{
                    mt: 0.5,
                    display: "flex",
                    gap: 1.5,
                    color: "text.secondary",
                    fontSize: "0.75rem",
                  }}
                >
                  <span>
                    Source: {MEMORY_SOURCE_LABELS[memory.provenance.source_kind]}
                  </span>
                  <span>Updated {relative(memory.updated_at)}</span>
                  {memory.access_count > 0 && (
                    <span>Used {memory.access_count}×</span>
                  )}
                </Box>
              </CardContent>
            </Card>
          ))}
        </Stack>
      </Box>

      {/* Floating add */}
      <Fab
        color="primary"
        aria-label="Add memory"
        onClick={openCreate}
        sx={{
          position: "absolute",
          right: 20,
          bottom: `calc(24px + env(safe-area-inset-bottom))`,
          bgcolor: "#7C3AED",
          "&:hover": { bgcolor: "#6D28D9" },
        }}
      >
        <AddRoundedIcon />
      </Fab>

      {/* Detail sheet */}
      <Dialog
        open={!!detail}
        onClose={() => setDetail(null)}
        fullWidth
        maxWidth="sm"
      >
        {detail && (
          <>
            <DialogTitle sx={{ pb: 0.5 }}>
              <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                <Chip
                  label={MEMORY_KIND_LABELS[detail.kind]}
                  size="small"
                  sx={{
                    bgcolor: `${KIND_COLORS[detail.kind]}15`,
                    color: KIND_COLORS[detail.kind],
                    fontWeight: 600,
                  }}
                />
                <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
                  Memory
                </Typography>
              </Box>
            </DialogTitle>
            <DialogContent>
              <Typography variant="body1" sx={{ mb: 2, fontWeight: 500 }}>
                {detail.content}
              </Typography>
              <Stack spacing={1}>
                <Row
                  label="Importance"
                  value={`${importanceLabel(detail.importance)} (${detail.importance}/5)`}
                />
                <Row
                  label="Confidence"
                  value={`${Math.round(detail.confidence * 100)}%`}
                />
                <Row
                  label="Source"
                  value={
                    MEMORY_SOURCE_LABELS[detail.provenance.source_kind] +
                    (detail.provenance.source_ref
                      ? ` · ${detail.provenance.source_ref}`
                      : "")
                  }
                />
                <Row label="Created" value={formatExact(detail.created_at)} />
                <Row label="Updated" value={formatExact(detail.updated_at)} />
                <Row
                  label="Last used"
                  value={
                    detail.last_accessed_at
                      ? `${formatExact(detail.last_accessed_at)} (${detail.access_count} time${detail.access_count === 1 ? "" : "s"})`
                      : "never"
                  }
                />
                {detail.expires_at && (
                  <Row label="Expires" value={formatExact(detail.expires_at)} />
                )}
                {detail.archived_at && (
                  <Row
                    label="Archived"
                    value={formatExact(detail.archived_at)}
                  />
                )}
                {detail.superseded_by && (
                  <Row
                    label="Superseded by"
                    value={detail.superseded_by}
                  />
                )}
              </Stack>
            </DialogContent>
            <DialogActions sx={{ px: 3, pb: 2 }}>
              <Button
                startIcon={<EditOutlinedIcon />}
                onClick={() => openEdit(detail)}
                disabled={detail.lifecycle === "superseded"}
              >
                Edit
              </Button>
              {detail.lifecycle === "active" ? (
                <Button
                  startIcon={<ArchiveOutlinedIcon />}
                  onClick={() => handleArchive(detail)}
                  color="warning"
                >
                  Archive
                </Button>
              ) : detail.lifecycle === "archived" ? (
                <Button
                  startIcon={<UnarchiveOutlinedIcon />}
                  onClick={() => handleRestore(detail)}
                  color="success"
                >
                  Restore
                </Button>
              ) : null}
              <Button onClick={() => setDetail(null)}>Close</Button>
            </DialogActions>
          </>
        )}
      </Dialog>

      {/* Edit dialog */}
      <Dialog
        open={!!editing}
        onClose={() => setEditing(null)}
        fullWidth
        maxWidth="sm"
      >
        <DialogTitle>Edit memory</DialogTitle>
        <DialogContent>
          <DraftForm draft={editDraft} onChange={setEditDraft} />
        </DialogContent>
        <DialogActions sx={{ px: 3, pb: 2 }}>
          <Button onClick={() => setEditing(null)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={handleEditSave}
            disabled={!editDraft.content.trim()}
          >
            Save
          </Button>
        </DialogActions>
      </Dialog>

      {/* Create dialog */}
      <Dialog
        open={creating}
        onClose={() => setCreating(false)}
        fullWidth
        maxWidth="sm"
      >
        <DialogTitle>Add a memory</DialogTitle>
        <DialogContent>
          <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 1 }}>
            Anything you enter here is stored as an explicit user input.
          </Typography>
          <DraftForm draft={createDraft} onChange={setCreateDraft} />
        </DialogContent>
        <DialogActions sx={{ px: 3, pb: 2 }}>
          <Button onClick={() => setCreating(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={handleCreate}
            disabled={!createDraft.content.trim()}
          >
            Save
          </Button>
        </DialogActions>
      </Dialog>

      <Snackbar
        open={!!notice}
        autoHideDuration={2400}
        onClose={() => setNotice(null)}
        message={notice ?? ""}
      />
    </Box>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <Box sx={{ display: "flex", gap: 1.5 }}>
      <Typography
        variant="caption"
        sx={{ width: 120, color: "text.secondary", flexShrink: 0 }}
      >
        {label}
      </Typography>
      <Typography variant="body2" sx={{ wordBreak: "break-word" }}>
        {value}
      </Typography>
    </Box>
  );
}

function DraftForm({
  draft,
  onChange,
}: {
  draft: DraftMemory;
  onChange: (next: DraftMemory) => void;
}) {
  return (
    <Stack spacing={2} sx={{ mt: 1 }}>
      <TextField
        select
        label="Kind"
        value={draft.kind}
        onChange={(e) =>
          onChange({ ...draft, kind: e.target.value as MemoryKind })
        }
        size="small"
      >
        {KIND_ORDER.map((kind) => (
          <MenuItem key={kind} value={kind}>
            {MEMORY_KIND_LABELS[kind]}
          </MenuItem>
        ))}
      </TextField>
      <TextField
        label="Content"
        value={draft.content}
        onChange={(e) => onChange({ ...draft, content: e.target.value })}
        multiline
        minRows={2}
        size="small"
        helperText="What should the assistant remember? Avoid pasting credentials."
      />
      <Box>
        <Typography variant="caption" color="text.secondary">
          Importance: {importanceLabel(draft.importance)}
        </Typography>
        <Slider
          value={draft.importance}
          min={1}
          max={5}
          step={1}
          marks
          size="small"
          onChange={(_, value) =>
            onChange({
              ...draft,
              importance: Array.isArray(value) ? value[0] : value,
            })
          }
        />
      </Box>
      <Box>
        <Typography variant="caption" color="text.secondary">
          Confidence: {Math.round(draft.confidence * 100)}%
        </Typography>
        <Slider
          value={draft.confidence}
          min={0}
          max={1}
          step={0.05}
          size="small"
          onChange={(_, value) =>
            onChange({
              ...draft,
              confidence: Array.isArray(value) ? value[0] : value,
            })
          }
        />
      </Box>
      {draft.kind === "temporary" && (
        <TextField
          type="datetime-local"
          label="Expires at"
          value={draft.expiresAt}
          onChange={(e) => onChange({ ...draft, expiresAt: e.target.value })}
          size="small"
          slotProps={{ inputLabel: { shrink: true } }}
          helperText="Temporary memories expire automatically."
        />
      )}
    </Stack>
  );
}

