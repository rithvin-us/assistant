/**
 * Documents screen (M8).
 *
 * Real document manager, not an AI chat: list, search, upload, per-page
 * inspection, retry-on-failure. The model is not involved anywhere on this
 * screen. Everything you see comes from `GET /v1/documents*`.
 */

import { useEffect, useMemo, useRef, useState } from "react";
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
import Divider from "@mui/material/Divider";
import Alert from "@mui/material/Alert";
import CircularProgress from "@mui/material/CircularProgress";
import Snackbar from "@mui/material/Snackbar";
import Stack from "@mui/material/Stack";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import DescriptionOutlinedIcon from "@mui/icons-material/DescriptionOutlined";
import PictureAsPdfOutlinedIcon from "@mui/icons-material/PictureAsPdfOutlined";
import UploadFileRoundedIcon from "@mui/icons-material/UploadFileRounded";
import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";
import DeleteOutlineRoundedIcon from "@mui/icons-material/DeleteOutlineRounded";
import CloudDoneOutlinedIcon from "@mui/icons-material/CloudDoneOutlined";

import type {
  DocumentItem,
  DocumentPageItem,
  DocumentSearchHit,
} from "../api/types";
import {
  listDocuments,
  uploadDocument,
  getDocument,
  listPages,
  reprocessDocument,
  deleteDocument,
  searchPages,
  humaniseSize,
  stateLabel,
} from "../api/documents";

interface DocumentsScreenProps {
  onBack?: () => void;
}

const STATE_COLORS: Record<string, string> = {
  uploaded: "#9CA3AF",
  extracting: "#2563EB",
  ocr: "#F59E0B",
  verifying: "#7C3AED",
  indexed: "#10B981",
  failed: "#DC2626",
};

function stateColor(state: string): string {
  return STATE_COLORS[state] ?? "#6B7280";
}

function fileIcon(mime: string) {
  if (mime === "application/pdf") {
    return <PictureAsPdfOutlinedIcon sx={{ color: "#DC2626" }} />;
  }
  return <DescriptionOutlinedIcon sx={{ color: "#2563EB" }} />;
}

function relative(iso: string | null | undefined): string {
  if (!iso) return "—";
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return "—";
  const diff = Date.now() - t;
  const m = Math.round(diff / 60_000);
  if (m < 1) return "just now";
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  if (d < 30) return `${d}d ago`;
  return new Date(iso).toLocaleDateString();
}

export default function DocumentsScreen({ onBack }: DocumentsScreenProps) {
  const [documents, setDocuments] = useState<DocumentItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [searchQuery, setSearchQuery] = useState("");
  const [pageHits, setPageHits] = useState<DocumentSearchHit[]>([]);

  const [detail, setDetail] = useState<DocumentItem | null>(null);
  const [detailPages, setDetailPages] = useState<DocumentPageItem[]>([]);
  const [detailLoading, setDetailLoading] = useState(false);
  const [openPage, setOpenPage] = useState<DocumentPageItem | null>(null);

  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const reload = async () => {
    setLoading(true);
    setErrorMsg(null);
    try {
      const docs = await listDocuments({
        q: searchQuery.trim() || undefined,
        limit: 100,
      });
      setDocuments(docs);
      if (searchQuery.trim()) {
        const hits = await searchPages({ q: searchQuery, limit: 10 });
        setPageHits(hits);
      } else {
        setPageHits([]);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to load documents";
      setErrorMsg(message);
      setDocuments([]);
      setPageHits([]);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      setLoading(true);
      try {
        const docs = await listDocuments({
          q: searchQuery.trim() || undefined,
          limit: 100,
        });
        if (cancelled) return;
        setDocuments(docs);
        if (searchQuery.trim()) {
          const hits = await searchPages({ q: searchQuery, limit: 10 });
          if (!cancelled) setPageHits(hits);
        } else if (!cancelled) {
          setPageHits([]);
        }
        setErrorMsg(null);
      } catch (err) {
        if (!cancelled) {
          const message =
            err instanceof Error ? err.message : "Failed to load documents";
          setErrorMsg(message);
          setDocuments([]);
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [searchQuery]);

  const handleUpload = async (file: File) => {
    try {
      setNotice(`Uploading ${file.name}…`);
      const uploaded = await uploadDocument(file);
      setNotice(
        uploaded.processing_state === "failed"
          ? `Uploaded but processing failed: ${uploaded.processing_error ?? "unknown error"}`
          : `Uploaded ${uploaded.filename}.`,
      );
      await reload();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Upload failed";
      setErrorMsg(message);
      setNotice(null);
    }
  };

  const openDetail = async (doc: DocumentItem) => {
    setDetail(doc);
    setDetailPages([]);
    setDetailLoading(true);
    try {
      const [fresh, pages] = await Promise.all([
        getDocument(doc.id),
        listPages(doc.id),
      ]);
      setDetail(fresh);
      setDetailPages(pages);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to open document";
      setErrorMsg(message);
    } finally {
      setDetailLoading(false);
    }
  };

  const handleReprocess = async (doc: DocumentItem) => {
    try {
      const refreshed = await reprocessDocument(doc.id);
      setDetail(refreshed);
      const pages = await listPages(refreshed.id);
      setDetailPages(pages);
      setNotice("Reprocessed.");
      await reload();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Reprocess failed";
      setErrorMsg(message);
    }
  };

  const handleDelete = async (doc: DocumentItem) => {
    try {
      await deleteDocument(doc.id);
      setDetail(null);
      setNotice("Deleted.");
      await reload();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Delete failed";
      setErrorMsg(message);
    }
  };

  const totalPages = useMemo(
    () => documents.reduce((n, d) => n + (d.page_count ?? 0), 0),
    [documents],
  );

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
        <CloudDoneOutlinedIcon sx={{ color: "#2563EB" }} />
        <Box sx={{ flex: 1 }}>
          <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
            Documents
          </Typography>
          <Typography variant="caption" color="text.secondary">
            {loading
              ? "Loading…"
              : `${documents.length} document${documents.length === 1 ? "" : "s"}, ${totalPages} page${totalPages === 1 ? "" : "s"}`}
          </Typography>
        </Box>
      </Box>

      {/* Search */}
      <Box sx={{ px: 2, pt: 1.5, pb: 0.5 }}>
        <TextField
          fullWidth
          size="small"
          placeholder="Search filenames and page content"
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

      {/* Body */}
      <Box sx={{ flex: 1, overflowY: "auto", px: 2, pb: 12 }}>
        {errorMsg && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {errorMsg}
          </Alert>
        )}
        {loading && documents.length === 0 && (
          <Box sx={{ display: "flex", justifyContent: "center", py: 6 }}>
            <CircularProgress size={24} />
          </Box>
        )}

        {pageHits.length > 0 && (
          <>
            <Typography
              variant="overline"
              color="text.secondary"
              sx={{ letterSpacing: "0.06em" }}
            >
              Matches in pages
            </Typography>
            <Stack spacing={1} sx={{ mb: 2 }}>
              {pageHits.map((hit) => (
                <Card
                  key={`${hit.document_id}-${hit.page_number}`}
                  variant="outlined"
                  onClick={() => {
                    const doc = documents.find((d) => d.id === hit.document_id);
                    if (doc) void openDetail(doc);
                  }}
                  sx={{ cursor: "pointer" }}
                >
                  <CardContent sx={{ py: 1.25, "&:last-child": { pb: 1.25 } }}>
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      {hit.filename} · p. {hit.page_number}
                    </Typography>
                    <Typography
                      variant="caption"
                      color="text.secondary"
                      sx={{ display: "block", mb: 0.5 }}
                    >
                      {hit.extraction_method} · score {hit.score.toFixed(2)}
                    </Typography>
                    <Typography variant="body2">{hit.snippet}</Typography>
                  </CardContent>
                </Card>
              ))}
            </Stack>
          </>
        )}

        <Typography
          variant="overline"
          color="text.secondary"
          sx={{ letterSpacing: "0.06em" }}
        >
          Documents
        </Typography>
        {!loading && documents.length === 0 && !errorMsg && (
          <Box sx={{ textAlign: "center", py: 6, color: "text.secondary" }}>
            <Typography variant="body2">
              No documents yet. Tap the + button to upload one.
            </Typography>
          </Box>
        )}
        <Stack spacing={1}>
          {documents.map((doc) => (
            <Card
              key={doc.id}
              variant="outlined"
              onClick={() => void openDetail(doc)}
              sx={{ cursor: "pointer" }}
            >
              <CardContent
                sx={{
                  py: 1.25,
                  "&:last-child": { pb: 1.25 },
                  display: "flex",
                  gap: 1.25,
                  alignItems: "center",
                }}
              >
                {fileIcon(doc.mime_type)}
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <Typography
                    variant="body2"
                    sx={{ fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
                  >
                    {doc.filename}
                  </Typography>
                  <Typography
                    variant="caption"
                    sx={{ color: "text.secondary" }}
                  >
                    {humaniseSize(doc.size_bytes)} ·{" "}
                    {doc.page_count != null ? `${doc.page_count} pages · ` : ""}
                    {doc.source === "google_drive" ? "Drive · " : ""}
                    updated {relative(doc.updated_at)}
                  </Typography>
                </Box>
                <Chip
                  label={stateLabel(doc.processing_state)}
                  size="small"
                  sx={{
                    color: stateColor(doc.processing_state),
                    borderColor: stateColor(doc.processing_state),
                    height: 20,
                  }}
                  variant="outlined"
                />
              </CardContent>
            </Card>
          ))}
        </Stack>
      </Box>

      {/* Upload FAB */}
      <input
        ref={fileInputRef}
        type="file"
        hidden
        accept="application/pdf,text/plain,text/markdown,text/csv,text/html,application/json,application/xml"
        onChange={(e) => {
          const file = e.target.files?.[0];
          if (file) void handleUpload(file);
          if (e.target) e.target.value = "";
        }}
      />
      <Fab
        color="primary"
        onClick={() => fileInputRef.current?.click()}
        aria-label="Upload document"
        sx={{
          position: "absolute",
          right: 20,
          bottom: `calc(24px + env(safe-area-inset-bottom))`,
          bgcolor: "#2563EB",
          "&:hover": { bgcolor: "#1D4ED8" },
        }}
      >
        <UploadFileRoundedIcon />
      </Fab>

      {/* Detail dialog */}
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
                {fileIcon(detail.mime_type)}
                <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
                  {detail.filename}
                </Typography>
              </Box>
            </DialogTitle>
            <DialogContent>
              <Stack spacing={0.75}>
                <Row label="Type" value={detail.mime_type} />
                <Row label="Size" value={humaniseSize(detail.size_bytes)} />
                <Row
                  label="Source"
                  value={
                    detail.source === "google_drive"
                      ? `Google Drive${detail.source_ref ? ` (${detail.source_ref})` : ""}`
                      : detail.source === "local_upload"
                        ? "Local upload"
                        : "External"
                  }
                />
                <Row
                  label="State"
                  value={stateLabel(detail.processing_state)}
                />
                {detail.processing_error && (
                  <Alert severity="error">{detail.processing_error}</Alert>
                )}
                <Row
                  label="Pages"
                  value={
                    detail.page_count != null ? String(detail.page_count) : "—"
                  }
                />
                <Row label="Created" value={relative(detail.created_at)} />
                <Row label="Updated" value={relative(detail.updated_at)} />
                <Row
                  label="Processed"
                  value={relative(detail.processed_at ?? undefined)}
                />
              </Stack>

              <Divider sx={{ my: 2 }} />

              <Typography variant="overline" color="text.secondary">
                Pages
              </Typography>
              {detailLoading && detailPages.length === 0 && (
                <Box sx={{ display: "flex", justifyContent: "center", py: 3 }}>
                  <CircularProgress size={20} />
                </Box>
              )}
              {!detailLoading && detailPages.length === 0 && (
                <Typography
                  variant="body2"
                  color="text.secondary"
                  sx={{ py: 2 }}
                >
                  {detail.processing_state === "failed"
                    ? "No pages: processing failed."
                    : "No page content available."}
                </Typography>
              )}
              <Stack spacing={0.5}>
                {detailPages.map((page) => (
                  <Box
                    key={page.page_number}
                    onClick={() => setOpenPage(page)}
                    sx={{
                      cursor: "pointer",
                      p: 1,
                      borderRadius: 1,
                      "&:hover": { bgcolor: "#F5F5F5" },
                    }}
                  >
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      Page {page.page_number}
                      {page.confidence != null
                        ? ` · confidence ${Math.round(page.confidence * 100)}%`
                        : ""}
                    </Typography>
                    <Typography
                      variant="caption"
                      color="text.secondary"
                      sx={{ display: "block" }}
                    >
                      {page.extraction_method} · {page.char_count} chars
                    </Typography>
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                        display: "-webkit-box",
                        WebkitLineClamp: 2,
                        WebkitBoxOrient: "vertical",
                        overflow: "hidden",
                      }}
                    >
                      {page.content || "(no text extracted for this page)"}
                    </Typography>
                  </Box>
                ))}
              </Stack>
            </DialogContent>
            <DialogActions sx={{ px: 3, pb: 2 }}>
              <Button
                startIcon={<RefreshRoundedIcon />}
                onClick={() => void handleReprocess(detail)}
              >
                Reprocess
              </Button>
              <Button
                startIcon={<DeleteOutlineRoundedIcon />}
                color="error"
                onClick={() => void handleDelete(detail)}
              >
                Delete
              </Button>
              <Button onClick={() => setDetail(null)}>Close</Button>
            </DialogActions>
          </>
        )}
      </Dialog>

      {/* Page-open dialog */}
      <Dialog
        open={!!openPage}
        onClose={() => setOpenPage(null)}
        fullWidth
        maxWidth="sm"
      >
        {openPage && (
          <>
            <DialogTitle>
              Page {openPage.page_number} · {openPage.extraction_method}
            </DialogTitle>
            <DialogContent>
              <Typography
                variant="body2"
                sx={{ whiteSpace: "pre-wrap", fontFamily: "serif" }}
              >
                {openPage.content || "(no text extracted for this page)"}
              </Typography>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setOpenPage(null)}>Close</Button>
            </DialogActions>
          </>
        )}
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
        sx={{ width: 100, color: "text.secondary", flexShrink: 0 }}
      >
        {label}
      </Typography>
      <Typography variant="body2" sx={{ wordBreak: "break-word" }}>
        {value}
      </Typography>
    </Box>
  );
}
