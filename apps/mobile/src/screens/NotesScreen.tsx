/**
 * NotesScreen â€” Apple Notes-inspired design.
 *
 * Three-view state machine: folders -> note list -> full-screen editor.
 * Design tokens from DESIGN-applenotes.md:
 *   - Cream canvas: #FFFBED (warm, not pure white)
 *   - Orange: #F09A38 (FAB, focus ring, tag chips)
 *   - Folder yellow: #F5D773 (folder glyph CSS)
 *   - Text/Ink: #1C1C1E / Slate: #8E8E93
 */

import { useState, useEffect, useRef, useCallback } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import IconButton from "@mui/material/IconButton";
import InputBase from "@mui/material/InputBase";
import Fab from "@mui/material/Fab";
import Divider from "@mui/material/Divider";

import ArrowBackIosNewRoundedIcon from "@mui/icons-material/ArrowBackIosNewRounded";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import EditRoundedIcon from "@mui/icons-material/EditRounded";
import PushPinRoundedIcon from "@mui/icons-material/PushPinRounded";
import KeyboardArrowRightRoundedIcon from "@mui/icons-material/KeyboardArrowRightRounded";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import DeleteOutlineRoundedIcon from "@mui/icons-material/DeleteOutlineRounded";
import ArchiveOutlinedIcon from "@mui/icons-material/ArchiveOutlined";
import UnarchiveOutlinedIcon from "@mui/icons-material/UnarchiveOutlined";
import ChecklistRoundedIcon from "@mui/icons-material/ChecklistRounded";
import TextFormatRoundedIcon from "@mui/icons-material/TextFormatRounded";
import MoreHorizRoundedIcon from "@mui/icons-material/MoreHorizRounded";
import MicRoundedIcon from "@mui/icons-material/MicRounded";

import type { NoteItem } from "../api/types";
import { fetchNotes, createNote, updateNote, deleteNote } from "../api/productivity";

// â”€â”€â”€ Color Tokens â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
const C = {
  cream: "#FFFBED",
  creamSurf1: "#FAF6E3",
  creamSurf2: "#F2EDD6",
  divider: "#EDEAD8",
  orange: "#F09A38",
  orangePressed: "#D87E1F",
  orangeTint: "#FFF1DD",
  folderYellow: "#F5D773",
  folderHighlight: "#FAE8A0",
  ink: "#1C1C1E",
  slate: "#8E8E93",
  mute: "#C7C7CC",
};

type View = "folders" | "list" | "editor";

interface FolderDef {
  id: string;
  label: string;
  smart: boolean;
  filter: (n: NoteItem) => boolean;
}

interface NotesScreenProps {
  onBack?: () => void;
}

// â”€â”€â”€ Folder Glyph (CSS-drawn yellow tab-folder) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
function FolderGlyph({ size = 24 }: { size?: number }) {
  return (
    <Box sx={{ width: size, height: size * 0.78, position: "relative", transform: "rotate(-2deg)", flexShrink: 0 }}>
      <Box sx={{ position: "absolute", bottom: 0, left: 0, right: 0, height: "82%", bgcolor: C.folderYellow, borderRadius: "0 3px 3px 3px", border: `1px solid ${C.orange}` }} />
      <Box sx={{ position: "absolute", top: 0, left: 0, width: "42%", height: "25%", bgcolor: C.folderYellow, borderRadius: "3px 6px 0 0", border: `1px solid ${C.orange}`, borderBottom: "none" }} />
      <Box sx={{ position: "absolute", top: "23%", left: 4, right: 4, height: 1, bgcolor: C.folderHighlight }} />
    </Box>
  );
}

// â”€â”€â”€ Note Row â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
function NoteRow({ note, onClick, onDelete, onToggleArchive, onTogglePin }: {
  note: NoteItem;
  onClick: () => void;
  onDelete: () => void;
  onToggleArchive: () => void;
  onTogglePin: () => void;
}) {
  const [pressed, setPressed] = useState(false);
  const [swipeX, setSwipeX] = useState(0);
  const touchStartX = useRef(0);

  const dateStr = note.updated_at
    ? new Date(note.updated_at).toLocaleDateString("en-US", { month: "short", day: "numeric" })
    : "";
  const preview = (note.content ?? "").replace(/\n/g, " ").trim();

  return (
    <Box sx={{ position: "relative", overflow: "hidden", bgcolor: note.is_pinned ? C.creamSurf1 : C.cream }}>
      <Box sx={{ position: "absolute", right: 0, top: 0, bottom: 0, width: 80, bgcolor: "#FF3B30", display: "flex", alignItems: "center", justifyContent: "center" }}>
        <DeleteOutlineRoundedIcon sx={{ color: "#fff", fontSize: 22 }} />
      </Box>
      <Box
        onTouchStart={(e) => { touchStartX.current = e.touches[0].clientX; }}
        onTouchMove={(e) => {
          const dx = e.touches[0].clientX - touchStartX.current;
          if (dx < 0) setSwipeX(Math.max(dx, -80));
        }}
        onTouchEnd={() => { if (swipeX < -50) onDelete(); setSwipeX(0); }}
        onMouseDown={() => setPressed(true)}
        onMouseUp={() => setPressed(false)}
        onMouseLeave={() => setPressed(false)}
        onClick={onClick}
        sx={{
          transform: `translateX(${swipeX}px) scale(${pressed ? 0.99 : 1})`,
          transition: swipeX === 0 ? "transform 0.2s ease" : "none",
          bgcolor: note.is_pinned ? C.creamSurf1 : C.cream,
          px: 2, pt: 1.75, pb: 0, cursor: "pointer", userSelect: "none",
        }}
      >
        <Box sx={{ minHeight: 68 }}>
          <Box sx={{ display: "flex", alignItems: "center", gap: 0.75, mb: 0.5 }}>
            {note.is_pinned && <PushPinRoundedIcon sx={{ fontSize: 11, color: C.orange, flexShrink: 0 }} />}
            <Typography sx={{ fontWeight: 600, fontSize: "0.94rem", lineHeight: 1.3, color: C.ink, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1 }}>
              {note.title || "New Note"}
            </Typography>
            <Box sx={{ display: "flex", gap: 0, ml: 0.5 }} onClick={(e) => e.stopPropagation()}>
              <IconButton size="small" sx={{ p: 0.5 }} onClick={onTogglePin}>
                <PushPinRoundedIcon sx={{ fontSize: 15, color: note.is_pinned ? C.orange : C.mute }} />
              </IconButton>
              <IconButton size="small" sx={{ p: 0.5 }} onClick={onToggleArchive}>
                {note.is_archived ? <UnarchiveOutlinedIcon sx={{ fontSize: 15, color: C.slate }} /> : <ArchiveOutlinedIcon sx={{ fontSize: 15, color: C.slate }} />}
              </IconButton>
            </Box>
          </Box>
          <Box sx={{ display: "flex", alignItems: "baseline", gap: 1 }}>
            <Typography sx={{ fontSize: "0.8rem", lineHeight: 1.35, color: C.slate, flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {preview || "No additional text"}
            </Typography>
            <Typography sx={{ fontSize: "0.72rem", color: C.slate, flexShrink: 0 }}>{dateStr}</Typography>
          </Box>
          {note.tags && note.tags.length > 0 && (
            <Box sx={{ display: "flex", gap: 0.5, mt: 0.75, flexWrap: "wrap" }}>
              {note.tags.map((tag) => (
                <Box key={tag} sx={{ height: 20, px: 1, borderRadius: 10, bgcolor: C.orangeTint, display: "flex", alignItems: "center" }}>
                  <Typography sx={{ fontSize: "0.67rem", color: C.orange, fontWeight: 500 }}>#{tag}</Typography>
                </Box>
              ))}
            </Box>
          )}
        </Box>
        <Box sx={{ height: 0.5, bgcolor: C.divider, mt: 1.5 }} />
      </Box>
    </Box>
  );
}

// â”€â”€â”€ Pinned Card â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
function PinnedCard({ note, onClick }: { note: NoteItem; onClick: () => void }) {
  return (
    <Box onClick={onClick} sx={{ width: 150, height: 90, flexShrink: 0, borderRadius: 2, bgcolor: C.creamSurf1, p: 1.5, cursor: "pointer", display: "flex", flexDirection: "column", gap: 0.5, border: `1px solid ${C.divider}`, transition: "transform 0.12s ease", "&:active": { transform: "scale(0.97)" } }}>
      <Box sx={{ display: "flex", alignItems: "flex-start", gap: 0.5 }}>
        <Typography sx={{ fontWeight: 600, fontSize: "0.82rem", color: C.ink, flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", lineHeight: 1.3 }}>
          {note.title || "New Note"}
        </Typography>
        <PushPinRoundedIcon sx={{ fontSize: 10, color: C.orange, flexShrink: 0, mt: 0.25 }} />
      </Box>
      <Typography sx={{ fontSize: "0.7rem", color: C.slate, lineHeight: 1.35, display: "-webkit-box", WebkitLineClamp: 3, WebkitBoxOrient: "vertical", overflow: "hidden" }}>
        {note.content || "No additional text"}
      </Typography>
    </Box>
  );
}

// â”€â”€â”€ Search Bar â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
function NotesSearchBar({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [focused, setFocused] = useState(false);
  return (
    <Box sx={{ display: "flex", alignItems: "center", height: 36, borderRadius: "10px", bgcolor: C.creamSurf1, border: `${focused ? 2 : 0}px solid ${focused ? C.orange : "transparent"}`, px: 1.5, gap: 1, transition: "border 0.15s ease" }}>
      <SearchRoundedIcon sx={{ fontSize: 15, color: C.slate, flexShrink: 0 }} />
      <InputBase
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="Search"
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        sx={{ flex: 1, fontSize: "0.95rem", color: C.ink, "& input": { p: 0 }, "& input::placeholder": { color: C.slate } }}
      />
      {value && (
        <IconButton size="small" sx={{ p: 0 }} onClick={() => onChange("")}>
          <Box sx={{ width: 16, height: 16, borderRadius: "50%", bgcolor: C.mute, display: "flex", alignItems: "center", justifyContent: "center" }}>
            <Typography sx={{ fontSize: "0.6rem", color: "#fff", lineHeight: 1 }}>x</Typography>
          </Box>
        </IconButton>
      )}
      <MicRoundedIcon sx={{ fontSize: 15, color: C.slate, flexShrink: 0 }} />
    </Box>
  );
}

// â”€â”€â”€ Note Editor â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
function NoteEditor({ note, onBack, onSave }: {
  note: Partial<NoteItem> & { isNew?: boolean };
  onBack: () => void;
  onSave: (title: string, content: string, tags: string[]) => Promise<void>;
}) {
  const [title, setTitle] = useState(note.title ?? "");
  const [body, setBody] = useState(note.content ?? "");
  const [saving, setSaving] = useState(false);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const dateStr = note.updated_at
    ? new Date(note.updated_at).toLocaleString("en-US", { month: "long", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit" })
    : new Date().toLocaleString("en-US", { month: "long", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit" });

  const scheduleAutosave = useCallback((t: string, b: string) => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(async () => {
      setSaving(true);
      try { await onSave(t, b, note.tags ?? []); } finally { setSaving(false); }
    }, 600);
  }, [note.tags, onSave]);

  useEffect(() => () => { if (saveTimer.current) clearTimeout(saveTimer.current); }, []);

  return (
    <Box sx={{ height: "100%", display: "flex", flexDirection: "column", bgcolor: C.cream }}>
      {/* Top bar */}
      <Box sx={{ display: "flex", alignItems: "center", px: 1, pt: 1, pb: 0.5, gap: 0.5 }}>
        <IconButton onClick={onBack} sx={{ color: C.orange }}>
          <ArrowBackIosNewRoundedIcon sx={{ fontSize: 18 }} />
        </IconButton>
        <Box sx={{ flex: 1 }} />
        <Typography sx={{ fontSize: "0.72rem", color: C.slate }}>{saving ? "Saving..." : "Auto-saved"}</Typography>
        <IconButton sx={{ color: C.slate }}><MoreHorizRoundedIcon sx={{ fontSize: 20 }} /></IconButton>
      </Box>
      <Typography sx={{ fontSize: "0.72rem", color: C.slate, textAlign: "center", pb: 0.5 }}>{dateStr}</Typography>
      <Divider sx={{ borderColor: C.divider }} />
      {/* Canvas */}
      <Box sx={{ flex: 1, overflowY: "auto", px: 2.5, pt: 2, pb: 10, display: "flex", flexDirection: "column", gap: 0.5 }}>
        <InputBase
          value={title}
          onChange={(e) => { setTitle(e.target.value); scheduleAutosave(e.target.value, body); }}
          placeholder="Title"
          multiline
          sx={{ "& textarea": { fontSize: "1.18rem", fontWeight: 600, lineHeight: 1.4, color: C.ink, p: 0, caretColor: C.orange, "&::placeholder": { color: C.mute } } }}
        />
        <InputBase
          value={body}
          onChange={(e) => { setBody(e.target.value); scheduleAutosave(title, e.target.value); }}
          placeholder="Start writing..."
          multiline
          minRows={12}
          sx={{ flex: 1, alignItems: "flex-start", "& textarea": { fontSize: "0.97rem", lineHeight: 1.65, color: C.ink, p: 0, caretColor: C.orange, "&::placeholder": { color: C.mute } } }}
        />
      </Box>
      {/* Formatting toolbar */}
      <Box sx={{ display: "flex", alignItems: "center", borderTop: `1px solid ${C.divider}`, bgcolor: C.cream, px: 1, py: 0.75, gap: 1, pb: "calc(8px + env(safe-area-inset-bottom))" }}>
        {[
          { icon: <ChecklistRoundedIcon />, label: "Checklist" },
          { icon: <TextFormatRoundedIcon />, label: "Format" },
          { icon: <MoreHorizRoundedIcon />, label: "More" },
        ].map(({ icon, label }) => (
          <IconButton key={label} size="small" aria-label={label} sx={{ color: C.slate, flex: 1 }}>{icon}</IconButton>
        ))}
      </Box>
    </Box>
  );
}

// â”€â”€â”€ Main Screen â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
export default function NotesScreen({ onBack }: NotesScreenProps) {
  const [view, setView] = useState<View>("folders");
  const [activeFolder, setActiveFolder] = useState<FolderDef | null>(null);
  const [notes, setNotes] = useState<NoteItem[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [editingNote, setEditingNote] = useState<(Partial<NoteItem> & { isNew?: boolean }) | null>(null);
  const [fabPressed, setFabPressed] = useState(false);

  const folders: FolderDef[] = [
    { id: "all", label: "All Notes", smart: false, filter: (n) => !n.is_archived },
    { id: "pinned", label: "Pinned", smart: true, filter: (n) => !n.is_archived && !!n.is_pinned },
    { id: "archived", label: "Recently Deleted", smart: true, filter: (n) => !!n.is_archived },
  ];

  const loadNotes = useCallback(async () => {
    try {
      const res = await fetchNotes({ q: searchQuery || undefined });
      setNotes(res);
    } catch (err) {
      console.warn("Failed to load notes:", err);
    }
  }, [searchQuery]);

  useEffect(() => {
    void loadNotes();
    const iv = setInterval(() => void loadNotes(), 15000);
    return () => clearInterval(iv);
  }, [loadNotes]);

  const folderNotes = (folder: FolderDef | null) => folder ? notes.filter(folder.filter) : [];

  const listNotes = folderNotes(activeFolder);
  const pinnedNotes = listNotes.filter((n) => n.is_pinned);
  const unpinnedNotes = listNotes.filter((n) => !n.is_pinned);
  const filteredNotes = searchQuery
    ? listNotes.filter((n) =>
        n.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
        (n.content ?? "").toLowerCase().includes(searchQuery.toLowerCase())
      )
    : null;

  const handleOpenFolder = (folder: FolderDef) => {
    setActiveFolder(folder);
    setSearchQuery("");
    setView("list");
  };

  const handleOpenNote = (note: NoteItem) => {
    setEditingNote(note);
    setView("editor");
  };

  const handleNewNote = () => {
    setEditingNote({ isNew: true, title: "", content: "", tags: [] });
    setView("editor");
  };

  const handleSaveNote = async (title: string, content: string, tags: string[]) => {
    if (!title.trim() && !content.trim()) return;
    if (editingNote?.id) {
      await updateNote(editingNote.id, { title, content, tags });
    } else {
      await createNote({ title, content, tags });
    }
    await loadNotes();
  };

  const handleDelete = async (id: string) => {
    setNotes((prev) => prev.filter((n) => n.id !== id));
    await deleteNote(id);
  };

  const handleToggleArchive = async (note: NoteItem) => {
    await updateNote(note.id, { is_archived: !note.is_archived });
    await loadNotes();
  };

  const handleTogglePin = async (note: NoteItem) => {
    await updateNote(note.id, { is_pinned: !note.is_pinned });
    await loadNotes();
  };

  const FabButton = () => (
    <Fab
      aria-label="New Note"
      onClick={handleNewNote}
      onMouseDown={() => setFabPressed(true)}
      onMouseUp={() => setFabPressed(false)}
      onTouchStart={() => setFabPressed(true)}
      onTouchEnd={() => setFabPressed(false)}
      sx={{
        position: "fixed",
        right: 20,
        bottom: "calc(20px + env(safe-area-inset-bottom))",
        bgcolor: C.orange,
        color: "#fff",
        boxShadow: `0 4px 20px ${C.orange}60`,
        transform: fabPressed ? "scale(0.94)" : "scale(1)",
        transition: "transform 0.15s cubic-bezier(0.34,1.56,0.64,1), background 0.1s",
        "&:hover": { bgcolor: C.orangePressed },
      }}
    >
      <EditRoundedIcon />
    </Fab>
  );

  // â”€â”€ Folders View â”€â”€
  if (view === "folders") {
    return (
      <Box sx={{ height: "100%", display: "flex", flexDirection: "column", bgcolor: C.cream }}>
        <Box sx={{ px: 2.5, pt: 2, pb: 1, display: "flex", alignItems: "center" }}>
          {onBack && (
            <IconButton size="small" onClick={onBack} sx={{ mr: 1, color: C.orange }}>
              <ArrowBackIosNewRoundedIcon sx={{ fontSize: 18 }} />
            </IconButton>
          )}
          <Typography sx={{ fontSize: "2.1rem", fontWeight: 800, color: C.ink, letterSpacing: -0.5, lineHeight: 1.1, flex: 1 }}>
            Notes
          </Typography>
          <IconButton sx={{ color: C.orange }}><EditRoundedIcon sx={{ fontSize: 22 }} /></IconButton>
        </Box>
        <Box sx={{ px: 2, pb: 1.5 }}>
          <NotesSearchBar value={searchQuery} onChange={setSearchQuery} />
        </Box>
        <Box sx={{ flex: 1, overflowY: "auto", pb: 6 }}>
          <Typography sx={{ px: 2.5, fontSize: "1.25rem", fontWeight: 700, color: C.ink, pb: 0.75 }}>
            My Folders
          </Typography>
          <Box sx={{ bgcolor: C.cream }}>
            {folders.map((folder, i) => {
              const count = folderNotes(folder).length;
              return (
                <Box key={folder.id}>
                  <Box
                    onClick={() => handleOpenFolder(folder)}
                    sx={{ display: "flex", alignItems: "center", px: 2.5, py: 1.5, gap: 1.5, cursor: "pointer", bgcolor: C.cream, transition: "background 0.12s", "&:active": { bgcolor: C.creamSurf2 } }}
                  >
                    {folder.smart ? (
                      <AutoAwesomeRoundedIcon sx={{ fontSize: 20, color: C.orange }} />
                    ) : (
                      <FolderGlyph size={26} />
                    )}
                    <Typography sx={{ flex: 1, fontSize: "1.02rem", color: C.ink, lineHeight: 1.3 }}>{folder.label}</Typography>
                    <Typography sx={{ fontSize: "1.02rem", color: C.slate }}>{count}</Typography>
                    <KeyboardArrowRightRoundedIcon sx={{ fontSize: 18, color: C.mute }} />
                  </Box>
                  {i < folders.length - 1 && <Box sx={{ height: 0.5, bgcolor: C.divider, ml: 6.5 }} />}
                </Box>
              );
            })}
          </Box>
          <Typography sx={{ textAlign: "center", pt: 3, fontSize: "0.8rem", color: C.slate }}>
            {notes.filter((n) => !n.is_archived).length} Notes
          </Typography>
        </Box>
        <FabButton />
      </Box>
    );
  }

  // â”€â”€ List View â”€â”€
  if (view === "list" && activeFolder) {
    const displayNotes = filteredNotes ?? unpinnedNotes;
    const displayPinned = filteredNotes ? [] : pinnedNotes;

    return (
      <Box sx={{ height: "100%", display: "flex", flexDirection: "column", bgcolor: C.cream }}>
        <Box sx={{ px: 1.5, pt: 1.5, pb: 0.5, display: "flex", alignItems: "center" }}>
          <IconButton onClick={() => { setView("folders"); setSearchQuery(""); }} sx={{ color: C.orange }}>
            <ArrowBackIosNewRoundedIcon sx={{ fontSize: 18 }} />
          </IconButton>
          <Box sx={{ flex: 1, display: "flex", justifyContent: "center" }}>
            <Typography sx={{ fontSize: "1.05rem", fontWeight: 700, color: C.ink }}>{activeFolder.label}</Typography>
          </Box>
          <IconButton onClick={handleNewNote} sx={{ color: C.orange }} aria-label="New note">
            <EditRoundedIcon sx={{ fontSize: 22 }} />
          </IconButton>
        </Box>
        <Box sx={{ px: 2, pb: 1 }}>
          <NotesSearchBar value={searchQuery} onChange={setSearchQuery} />
        </Box>
        <Box sx={{ flex: 1, overflowY: "auto", pb: 6 }}>
          {displayPinned.length > 0 && (
            <Box sx={{ pb: 1.5 }}>
              <Typography sx={{ px: 2.5, fontSize: "0.78rem", fontWeight: 700, color: C.slate, pb: 0.75, textTransform: "uppercase", letterSpacing: 0.5 }}>
                Pinned
              </Typography>
              <Box sx={{ display: "flex", gap: 1.25, overflowX: "auto", px: 2, pb: 1, scrollbarWidth: "none", "&::-webkit-scrollbar": { display: "none" } }}>
                {displayPinned.map((note) => (
                  <PinnedCard key={note.id} note={note} onClick={() => handleOpenNote(note)} />
                ))}
              </Box>
              <Divider sx={{ borderColor: C.divider }} />
            </Box>
          )}
          {displayNotes.length === 0 && displayPinned.length === 0 ? (
            <Typography sx={{ textAlign: "center", pt: 8, color: C.slate, fontSize: "0.9rem" }}>
              {searchQuery ? "No notes match your search." : "No notes yet. Tap the pencil to write one."}
            </Typography>
          ) : (
            <>
              {displayPinned.length > 0 && displayNotes.length > 0 && (
                <Typography sx={{ px: 2.5, fontSize: "0.78rem", fontWeight: 700, color: C.slate, pb: 0.75, pt: 0.5, textTransform: "uppercase", letterSpacing: 0.5 }}>
                  Notes
                </Typography>
              )}
              {displayNotes.map((note) => (
                <NoteRow
                  key={note.id}
                  note={note}
                  onClick={() => handleOpenNote(note)}
                  onDelete={() => void handleDelete(note.id)}
                  onToggleArchive={() => void handleToggleArchive(note)}
                  onTogglePin={() => void handleTogglePin(note)}
                />
              ))}
            </>
          )}
          <Typography sx={{ textAlign: "center", pt: 2, fontSize: "0.78rem", color: C.mute }}>
            {listNotes.length} {listNotes.length === 1 ? "Note" : "Notes"}
          </Typography>
        </Box>
        <FabButton />
      </Box>
    );
  }

  // â”€â”€ Editor View â”€â”€
  if (view === "editor" && editingNote !== null) {
    return (
      <NoteEditor
        note={editingNote}
        onBack={() => {
          setView(activeFolder ? "list" : "folders");
          setEditingNote(null);
          void loadNotes();
        }}
        onSave={handleSaveNote}
      />
    );
  }

  return null;
}
