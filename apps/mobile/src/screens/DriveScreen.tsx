/**
 * Drive — search and browse, read-only.
 *
 * Provides file listing, search, automatic silent refresh, and an interactive in-app file previewer
 * supporting embedded Google Drive file viewer, inline text content reader, and swipe-down-to-close drawer.
 */

import { useCallback, useEffect, useState } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import IconButton from "@mui/material/IconButton";
import TextField from "@mui/material/TextField";
import InputAdornment from "@mui/material/InputAdornment";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemText from "@mui/material/ListItemText";
import ListItemIcon from "@mui/material/ListItemIcon";
import Divider from "@mui/material/Divider";
import Drawer from "@mui/material/Drawer";
import Button from "@mui/material/Button";
import Alert from "@mui/material/Alert";
import CircularProgress from "@mui/material/CircularProgress";
import Tabs from "@mui/material/Tabs";
import Tab from "@mui/material/Tab";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import FolderRoundedIcon from "@mui/icons-material/FolderRounded";
import InsertDriveFileOutlinedIcon from "@mui/icons-material/InsertDriveFileOutlined";
import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import OpenInNewRoundedIcon from "@mui/icons-material/OpenInNewRounded";
import DescriptionOutlinedIcon from "@mui/icons-material/DescriptionOutlined";
import VisibilityOutlinedIcon from "@mui/icons-material/VisibilityOutlined";
import ContentCopyOutlinedIcon from "@mui/icons-material/ContentCopyOutlined";

import { fetchGoogleAccounts } from "../api/google";
import {
  fileKind,
  formatSize,
  listDrive,
  readDriveFile,
  searchDrive,
} from "../api/academic";
import AccountPicker from "../components/AccountPicker";
import { hasScopesFor } from "../api/scopes";
import type { AccountSummary, DriveFile, DriveFileContent } from "../api/types";

interface Props {
  onBack: () => void;
  onOpenConnections?: () => void;
}

export default function DriveScreen({ onBack, onOpenConnections }: Props) {
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [accountId, setAccountId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [files, setFiles] = useState<DriveFile[]>([]);
  const [folderStack, setFolderStack] = useState<DriveFile[]>([]);
  const [selected, setSelected] = useState<DriveFile | null>(null);
  const [content, setContent] = useState<DriveFileContent | null>(null);
  const [contentError, setContentError] = useState<string | null>(null);
  const [loadingContent, setLoadingContent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [previewTab, setPreviewTab] = useState<"embed" | "text">("embed");
  const [copied, setCopied] = useState(false);

  // Touch Swipe Down Handler State for Preview Drawer
  const [touchStartY, setTouchStartY] = useState<number | null>(null);
  const [dragOffsetY, setDragOffsetY] = useState<number>(0);

  const handleTouchStart = (e: React.TouchEvent) => {
    setTouchStartY(e.touches[0].clientY);
  };

  const handleTouchMove = (e: React.TouchEvent) => {
    if (touchStartY === null) return;
    const currentY = e.touches[0].clientY;
    const diffY = currentY - touchStartY;
    if (diffY > 0) {
      setDragOffsetY(diffY);
    }
  };

  const handleTouchEnd = () => {
    if (dragOffsetY > 100) {
      setSelected(null);
    }
    setTouchStartY(null);
    setDragOffsetY(0);
  };

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await fetchGoogleAccounts();
        if (cancelled) return;
        setAccounts(list);
        const active = list.find((a) => a.status === "active") ?? list[0];
        if (active) setAccountId(active.id);
      } catch (e: unknown) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const load = useCallback(async (id: string, folder?: DriveFile, silent = false) => {
    try {
      if (!silent) setBusy(true);
      const next = await listDrive(id, folder?.external_id);
      setFiles(next);
      setError(null);
    } catch (e: unknown) {
      if (!silent) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (!silent) setBusy(false);
    }
  }, []);

  useEffect(() => {
    if (!accountId || query.trim() !== "") return;
    let cancelled = false;
    const folder = folderStack[folderStack.length - 1];

    // eslint-disable-next-line react-hooks/set-state-in-effect
    void load(accountId, folder, false);

    const interval = setInterval(() => {
      if (!cancelled) void load(accountId, folder, true);
    }, 10000);

    const onFocus = () => {
      if (!cancelled) void load(accountId, folder, true);
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);

    return () => {
      cancelled = true;
      clearInterval(interval);
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, [accountId, folderStack, load, query]);

  const runSearch = useCallback(async (silent = false) => {
    if (!accountId) return;
    if (query.trim() === "") {
      await load(accountId, folderStack[folderStack.length - 1], silent);
      return;
    }
    if (!silent) setBusy(true);
    setError(null);
    try {
      setFiles(await searchDrive(accountId, query.trim()));
    } catch (e: unknown) {
      if (!silent) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (!silent) setBusy(false);
    }
  }, [accountId, folderStack, load, query]);

  const fetchTextContent = useCallback(async (file: DriveFile, accId: string) => {
    setLoadingContent(true);
    setContentError(null);
    setContent(null);
    try {
      const res = await readDriveFile(accId, file.external_id);
      setContent(res);
    } catch (e: unknown) {
      setContentError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoadingContent(false);
    }
  }, []);

  const openFile = useCallback((file: DriveFile) => {
    if (file.is_folder) {
      setQuery("");
      setFolderStack((s) => [...s, file]);
      return;
    }
    setSelected(file);
    setPreviewTab("embed");
    if (accountId) {
      void fetchTextContent(file, accountId);
    }
  }, [accountId, fetchTextContent]);

  const handleCopyText = async () => {
    if (content?.text) {
      try {
        await navigator.clipboard.writeText(content.text);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      } catch {
        // Fallback
      }
    }
  };

  const selectedAccount = accounts.find((a) => a.id === accountId);
  const canUse = hasScopesFor(selectedAccount, "drive");

  return (
    <Box sx={{ height: "100%", display: "flex", flexDirection: "column", bgcolor: "#FAFAFA" }}>
      {/* Header Bar */}
      <Box sx={{ display: "flex", alignItems: "center", gap: 1, px: 2, pt: 1.5, pb: 1, borderBottom: "1px solid #F0F0F0", bgcolor: "#FFFFFF" }}>
        <IconButton
          onClick={() => {
            if (folderStack.length > 0) setFolderStack((s) => s.slice(0, -1));
            else onBack();
          }}
          aria-label="Back"
          size="small"
        >
          <ArrowBackRoundedIcon />
        </IconButton>
        <Typography variant="h6" noWrap sx={{ flex: 1, fontWeight: 700, fontSize: "1.15rem" }}>
          {folderStack[folderStack.length - 1]?.name ?? "Drive"}
        </Typography>
        <IconButton
          size="small"
          onClick={() => {
            if (accountId) void load(accountId, folderStack[folderStack.length - 1], false);
          }}
          aria-label="Refresh"
        >
          <RefreshRoundedIcon fontSize="small" />
        </IconButton>
      </Box>

      {/* Account Switcher */}
      <AccountPicker
        accounts={accounts}
        selectedId={accountId}
        onSelect={setAccountId}
        feature="drive"
        onOpenConnections={onOpenConnections}
      />

      {/* Search Input */}
      <Box sx={{ px: 2, py: 1.5, bgcolor: "#FFFFFF" }}>
        <TextField
          fullWidth
          size="small"
          placeholder="Search files in Drive..."
          value={query}
          disabled={!canUse}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void runSearch(false);
          }}
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <SearchRoundedIcon fontSize="small" sx={{ color: "#888" }} />
                </InputAdornment>
              ),
            },
          }}
          sx={{
            "& .MuiOutlinedInput-root": {
              borderRadius: "10px",
              bgcolor: "#F8F9FA",
            },
          }}
        />
      </Box>

      {error && (
        <Alert severity="error" sx={{ mx: 2, my: 1, borderRadius: "10px" }} onClose={() => setError(null)}>
          {error}
        </Alert>
      )}

      {busy && (
        <Box sx={{ display: "flex", justifyContent: "center", py: 3 }}>
          <CircularProgress size={24} sx={{ color: "#2563EB" }} />
        </Box>
      )}

      {/* Drive File List */}
      <Box sx={{ flex: 1, overflowY: "auto" }}>
        {!busy && files.length === 0 && canUse && (
          <Typography variant="body2" sx={{ px: 3, py: 6, color: "text.secondary", textAlign: "center" }}>
            {query.trim() ? "No files matched your search." : "This folder is empty."}
          </Typography>
        )}

        {files.map((file) => (
          <Box key={file.external_id}>
            <ListItemButton onClick={() => openFile(file)} sx={{ py: 1.5, px: 2.5 }}>
              <ListItemIcon sx={{ minWidth: 42 }}>
                {file.is_folder ? (
                  <FolderRoundedIcon sx={{ color: "#F59E0B" }} />
                ) : (
                  <InsertDriveFileOutlinedIcon sx={{ color: "#2563EB" }} />
                )}
              </ListItemIcon>
              <ListItemText
                primary={file.name}
                secondary={[fileKind(file.mime_type), formatSize(file.size_bytes)]
                  .filter(Boolean)
                  .join(" · ")}
                slotProps={{
                  primary: { noWrap: true, sx: { fontWeight: 600, fontSize: "0.95rem", color: "#1E293B" } },
                  secondary: { sx: { fontSize: "0.8rem", color: "#64748B" } },
                }}
              />
            </ListItemButton>
            <Divider component="li" sx={{ listStyle: "none", ml: 7.5 }} />
          </Box>
        ))}
      </Box>

      {/* In-App File Preview Drawer with Swipe-down-to-close */}
      <Drawer
        anchor="bottom"
        open={selected !== null}
        onClose={() => setSelected(null)}
        slotProps={{
          paper: {
            sx: {
              height: "90vh",
              borderTopLeftRadius: 24,
              borderTopRightRadius: 24,
              bgcolor: "#FFFFFF",
              display: "flex",
              flexDirection: "column",
              transform: dragOffsetY > 0 ? `translateY(${dragOffsetY}px)` : "none",
              transition: dragOffsetY > 0 ? "none" : "transform 0.2s ease-out",
            },
          },
        }}
      >
        {selected && (
          <Box sx={{ height: "100%", display: "flex", flexDirection: "column" }}>
            {/* Top Swipe Drag Bar */}
            <Box
              onTouchStart={handleTouchStart}
              onTouchMove={handleTouchMove}
              onTouchEnd={handleTouchEnd}
              sx={{
                py: 1.2,
                px: 2,
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                cursor: "grab",
                bgcolor: "#F9FAFB",
                borderTopLeftRadius: 24,
                borderTopRightRadius: 24,
                borderBottom: "1px solid #F3F4F6",
                flexShrink: 0,
              }}
            >
              <Box
                sx={{
                  width: 44,
                  height: 5,
                  borderRadius: 3,
                  bgcolor: "#D1D5DB",
                  mb: 1,
                }}
              />
              <Box sx={{ width: "100%", display: "flex", alignItems: "center", justifyContent: "space-between" }}>
                <Typography variant="overline" sx={{ color: "#6B7280", fontWeight: 700, letterSpacing: 0.8 }}>
                  Pull down to close
                </Typography>
                <IconButton size="small" onClick={() => setSelected(null)}>
                  <CloseRoundedIcon fontSize="small" />
                </IconButton>
              </Box>
            </Box>

            {/* File Info Header */}
            <Box sx={{ px: 2.5, pt: 2, pb: 1.5, borderBottom: "1px solid #F1F5F9" }}>
              <Typography variant="h6" sx={{ fontWeight: 800, color: "#0F172A", fontSize: "1.1rem", lineHeight: 1.3, mb: 0.5 }}>
                {selected.name}
              </Typography>
              <Typography variant="body2" sx={{ color: "#64748B", fontSize: "0.825rem", mb: 1 }}>
                {fileKind(selected.mime_type)}
                {selected.size_bytes ? ` · ${formatSize(selected.size_bytes)}` : ""}
                {selected.modified_at
                  ? ` · ${new Date(selected.modified_at).toLocaleDateString()}`
                  : ""}
              </Typography>

              {/* Action Toolbar */}
              <Box sx={{ display: "flex", gap: 1, alignItems: "center", flexWrap: "wrap", mt: 1 }}>
                {selected.web_view_link && (
                  <Button
                    size="small"
                    variant="outlined"
                    startIcon={<OpenInNewRoundedIcon />}
                    component="a"
                    href={selected.web_view_link}
                    target="_blank"
                    rel="noreferrer"
                    sx={{ borderRadius: "8px", textTransform: "none", fontWeight: 600, fontSize: "0.8rem" }}
                  >
                    Open in Drive
                  </Button>
                )}
                {content?.text && (
                  <Button
                    size="small"
                    variant="text"
                    startIcon={<ContentCopyOutlinedIcon />}
                    onClick={() => void handleCopyText()}
                    sx={{ borderRadius: "8px", textTransform: "none", fontWeight: 600, fontSize: "0.8rem" }}
                  >
                    {copied ? "Copied!" : "Copy Text"}
                  </Button>
                )}
              </Box>
            </Box>

            {/* View Mode Selector Tabs */}
            <Box sx={{ borderBottom: 1, borderColor: "divider", bgcolor: "#FAFAFA", flexShrink: 0 }}>
              <Tabs
                value={previewTab}
                onChange={(_, v) => setPreviewTab(v as "embed" | "text")}
                sx={{ minHeight: 40 }}
              >
                <Tab
                  icon={<VisibilityOutlinedIcon sx={{ fontSize: 18 }} />}
                  iconPosition="start"
                  label="Document Preview"
                  value="embed"
                  sx={{ minHeight: 40, textTransform: "none", fontWeight: 600, fontSize: "0.85rem" }}
                />
                <Tab
                  icon={<DescriptionOutlinedIcon sx={{ fontSize: 18 }} />}
                  iconPosition="start"
                  label="Text Reader"
                  value="text"
                  sx={{ minHeight: 40, textTransform: "none", fontWeight: 600, fontSize: "0.85rem" }}
                />
              </Tabs>
            </Box>

            {/* Main Preview Container */}
            <Box sx={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden", bgcolor: "#FFFFFF" }}>
              {previewTab === "embed" && (
                <Box sx={{ width: "100%", height: "100%", flex: 1, display: "flex", flexDirection: "column", position: "relative" }}>
                  <iframe
                    title={selected.name}
                    src={`https://drive.google.com/file/d/${selected.external_id}/preview`}
                    style={{
                      width: "100%",
                      height: "100%",
                      border: "none",
                      flex: 1,
                    }}
                    allow="autoplay"
                  />
                </Box>
              )}

              {previewTab === "text" && (
                <Box sx={{ flex: 1, overflowY: "auto", p: 2.5 }}>
                  {loadingContent ? (
                    <Box sx={{ display: "flex", justifyContent: "center", py: 8 }}>
                      <CircularProgress size={26} sx={{ color: "#2563EB" }} />
                    </Box>
                  ) : contentError ? (
                    <Alert severity="info" sx={{ borderRadius: "10px" }}>
                      {contentError}
                    </Alert>
                  ) : content ? (
                    <>
                      {content.truncated && (
                        <Alert severity="warning" sx={{ mb: 2, borderRadius: "10px" }}>
                          Showing the first portion of this file.
                        </Alert>
                      )}
                      <Box
                        component="pre"
                        sx={{
                          m: 0,
                          p: 2,
                          bgcolor: "#F8FAFC",
                          border: "1px solid #E2E8F0",
                          borderRadius: "12px",
                          fontSize: 13,
                          fontFamily: "monospace",
                          lineHeight: 1.6,
                          whiteSpace: "pre-wrap",
                          wordBreak: "break-word",
                          color: "#1E293B",
                        }}
                      >
                        {content.text}
                      </Box>
                    </>
                  ) : (
                    <Typography variant="body2" sx={{ color: "text.secondary", fontStyle: "italic", textAlign: "center", py: 4 }}>
                      No text content loaded for this file.
                    </Typography>
                  )}
                </Box>
              )}
            </Box>
          </Box>
        )}
      </Drawer>
    </Box>
  );
}

