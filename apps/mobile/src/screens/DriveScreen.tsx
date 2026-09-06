/**
 * Drive — search and browse, read-only.
 *
 * Metadata only until the user asks for a file's contents, and even then the
 * server refuses anything oversized or binary. Nothing here mirrors a file
 * into the app's own storage.
 *
 * Internal Google identifiers are never shown. A file is a name, a kind, a
 * date and an account.
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
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import FolderRoundedIcon from "@mui/icons-material/FolderRounded";
import InsertDriveFileOutlinedIcon from "@mui/icons-material/InsertDriveFileOutlined";

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
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  const load = useCallback(async (id: string, folder?: DriveFile) => {
    try {
      const next = await listDrive(id, folder?.external_id);
      setFiles(next);
      setError(null);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    if (!accountId || query.trim() !== "") return;
    let cancelled = false;
    const folder = folderStack[folderStack.length - 1];
    void (async () => {
      if (cancelled) return;
      await load(accountId, folder);
    })();
    return () => {
      cancelled = true;
    };
  }, [accountId, folderStack, load, query]);

  const runSearch = useCallback(async () => {
    if (!accountId) return;
    if (query.trim() === "") {
      await load(accountId, folderStack[folderStack.length - 1]);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setFiles(await searchDrive(accountId, query.trim()));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [accountId, folderStack, load, query]);

  const openFile = useCallback((file: DriveFile) => {
    if (file.is_folder) {
      setQuery("");
      setFolderStack((s) => [...s, file]);
      return;
    }
    setSelected(file);
    setContent(null);
    setContentError(null);
  }, []);

  /**
   * Asks the server for the text. A refusal — too large, binary, a PDF — comes
   * back as an error and is shown as one. An empty document is never rendered
   * in its place.
   */
  const openContent = useCallback(async () => {
    if (!accountId || !selected) return;
    setBusy(true);
    setContentError(null);
    try {
      setContent(await readDriveFile(accountId, selected.external_id));
    } catch (e: unknown) {
      setContentError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [accountId, selected]);

  const selectedAccount = accounts.find((a) => a.id === accountId);
  const canUse = hasScopesFor(selectedAccount, "drive");

  return (
    <Box sx={{ height: "100%", display: "flex", flexDirection: "column" }}>
      <Box sx={{ display: "flex", alignItems: "center", gap: 1, px: 1, pt: 1 }}>
        <IconButton
          onClick={() => {
            if (folderStack.length > 0) setFolderStack((s) => s.slice(0, -1));
            else onBack();
          }}
          aria-label="Back"
        >
          <ArrowBackRoundedIcon />
        </IconButton>
        <Typography variant="h6" sx={{ flex: 1, fontWeight: 700 }}>
          {folderStack[folderStack.length - 1]?.name ?? "Drive"}
        </Typography>
      </Box>

      <AccountPicker
        accounts={accounts}
        selectedId={accountId}
        onSelect={setAccountId}
        feature="drive"
        onOpenConnections={onOpenConnections}
      />

      <Box sx={{ px: 2, pb: 1 }}>
        <TextField
          fullWidth
          size="small"
          placeholder="Search Drive"
          value={query}
          disabled={!canUse}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void runSearch();
          }}
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

      {error && (
        <Alert severity="error" sx={{ mx: 2, mb: 1 }} onClose={() => setError(null)}>
          {error}
        </Alert>
      )}

      {busy && (
        <Box sx={{ display: "flex", justifyContent: "center", py: 2 }}>
          <CircularProgress size={22} />
        </Box>
      )}

      <Box sx={{ flex: 1, overflowY: "auto" }}>
        {!busy && files.length === 0 && canUse && (
          <Typography variant="body2" sx={{ px: 3, py: 4, color: "text.secondary" }}>
            {query.trim() ? "Nothing matched that search." : "This folder is empty."}
          </Typography>
        )}

        {files.map((file) => (
          <Box key={file.external_id}>
            <ListItemButton onClick={() => openFile(file)}>
              <ListItemIcon sx={{ minWidth: 40 }}>
                {file.is_folder ? (
                  <FolderRoundedIcon fontSize="small" />
                ) : (
                  <InsertDriveFileOutlinedIcon fontSize="small" />
                )}
              </ListItemIcon>
              <ListItemText
                primary={file.name}
                secondary={[fileKind(file.mime_type), formatSize(file.size_bytes)]
                  .filter(Boolean)
                  .join(" · ")}
                slotProps={{ primary: { noWrap: true } }}
              />
            </ListItemButton>
            <Divider component="li" sx={{ listStyle: "none" }} />
          </Box>
        ))}
      </Box>

      <Drawer
        anchor="bottom"
        open={selected !== null}
        onClose={() => setSelected(null)}
        slotProps={{
          paper: {
            sx: { borderTopLeftRadius: 16, borderTopRightRadius: 16, maxHeight: "80%" },
          },
        }}
      >
        {selected && (
          <Box sx={{ p: 2.5 }}>
            <Typography variant="h6" sx={{ fontWeight: 700, mb: 0.5 }}>
              {selected.name}
            </Typography>
            <Typography variant="body2" sx={{ color: "text.secondary", mb: 0.5 }}>
              {fileKind(selected.mime_type)}
              {selected.size_bytes ? ` · ${formatSize(selected.size_bytes)}` : ""}
              {selected.modified_at
                ? ` · ${new Date(selected.modified_at).toLocaleDateString()}`
                : ""}
            </Typography>
            <Typography variant="caption" sx={{ color: "text.secondary", display: "block", mb: 2 }}>
              {selectedAccount?.email}
            </Typography>

            <Box sx={{ display: "flex", gap: 1, mb: 2 }}>
              <Button size="small" variant="outlined" onClick={() => void openContent()}>
                Read here
              </Button>
              {selected.web_view_link && (
                <Button
                  size="small"
                  component="a"
                  href={selected.web_view_link}
                  target="_blank"
                  rel="noreferrer"
                >
                  Open in Drive
                </Button>
              )}
            </Box>

            {contentError && (
              <Alert severity="info" sx={{ mb: 2 }}>
                {contentError}
              </Alert>
            )}

            {content && (
              <>
                {content.truncated && (
                  <Alert severity="info" sx={{ mb: 1 }}>
                    Showing the beginning of this file only.
                  </Alert>
                )}
                <Box
                  component="pre"
                  sx={{
                    m: 0,
                    p: 1.5,
                    bgcolor: "action.hover",
                    borderRadius: 1,
                    fontSize: 12,
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-word",
                    maxHeight: "40vh",
                    overflowY: "auto",
                  }}
                >
                  {content.text}
                </Box>
              </>
            )}
          </Box>
        )}
      </Drawer>
    </Box>
  );
}
