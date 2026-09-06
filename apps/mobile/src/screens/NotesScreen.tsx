/**
 * Standalone Notes Page.
 *
 * Implements Notes management with Todoist-inspired visual language.
 * Clean layout with zero connection bars or tabs at the top.
 */

import { useState, useEffect } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import TextField from "@mui/material/TextField";
import InputAdornment from "@mui/material/InputAdornment";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import Dialog from "@mui/material/Dialog";
import DialogTitle from "@mui/material/DialogTitle";
import DialogContent from "@mui/material/DialogContent";
import DialogActions from "@mui/material/DialogActions";
import Card from "@mui/material/Card";
import CardContent from "@mui/material/CardContent";
import IconButton from "@mui/material/IconButton";
import Fab from "@mui/material/Fab";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import AddRoundedIcon from "@mui/icons-material/AddRounded";
import DeleteOutlineRoundedIcon from "@mui/icons-material/DeleteOutlineRounded";
import ArchiveOutlinedIcon from "@mui/icons-material/ArchiveOutlined";
import UnarchiveOutlinedIcon from "@mui/icons-material/UnarchiveOutlined";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";

import type { NoteItem } from "../api/types";
import {
  fetchNotes,
  createNote,
  updateNote,
  deleteNote,
} from "../api/productivity";

interface NotesScreenProps {
  onBack?: () => void;
}

export default function NotesScreen({ onBack }: NotesScreenProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [notes, setNotes] = useState<NoteItem[]>([]);
  const [noteArchiveFilter, setNoteArchiveFilter] = useState<boolean>(false);

  const [noteModalOpen, setNoteModalOpen] = useState(false);
  const [editingNote, setEditingNote] = useState<NoteItem | null>(null);
  const [noteTitle, setNoteTitle] = useState("");
  const [noteContent, setNoteContent] = useState("");
  const [noteTagsInput, setNoteTagsInput] = useState("");

  const loadData = async () => {
    try {
      const res = await fetchNotes({
        is_archived: noteArchiveFilter,
        q: searchQuery || undefined,
      });
      setNotes(res);
    } catch (err) {
      console.warn("Failed to load notes:", err);
    }
  };

  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) return;
      void (async () => {
        try {
          const res = await fetchNotes({
            is_archived: noteArchiveFilter,
            q: searchQuery || undefined,
          });
          if (!cancelled) setNotes(res);
        } catch (err) {
          console.warn("Failed to load notes:", err);
        }
      })();
    });
    return () => {
      cancelled = true;
    };
  }, [noteArchiveFilter, searchQuery]);

  const handleSaveNote = async () => {
    if (!noteTitle.trim()) return;
    try {
      const tags = noteTagsInput
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      if (editingNote) {
        await updateNote(editingNote.id, {
          title: noteTitle,
          content: noteContent,
          tags,
        });
      } else {
        await createNote({
          title: noteTitle,
          content: noteContent,
          tags,
        });
      }
      setNoteModalOpen(false);
      setEditingNote(null);
      setNoteTitle("");
      setNoteContent("");
      setNoteTagsInput("");
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleToggleArchiveNote = async (note: NoteItem) => {
    try {
      await updateNote(note.id, { is_archived: !note.is_archived });
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleDeleteNote = async (id: string) => {
    try {
      setNotes((prev) => prev.filter((n) => n.id !== id));
      await deleteNote(id);
    } catch (err) {
      console.error(err);
    }
  };

  return (
    <Box
      sx={{
        height: "100%",
        display: "flex",
        flexDirection: "column",
        bgcolor: "#FFFFFF",
        color: "#202020",
      }}
    >
      {/* Header Bar — Clean header with no connection bar or tabs above */}
      <Box
        sx={{
          px: 2,
          pt: 1.5,
          pb: 1.5,
          display: "flex",
          alignItems: "center",
          gap: 1,
          borderBottom: "1px solid #EEEEEE",
        }}
      >
        {onBack && (
          <IconButton size="small" onClick={onBack} sx={{ color: "#202020" }}>
            <ArrowBackRoundedIcon />
          </IconButton>
        )}
        <Typography variant="h6" sx={{ fontWeight: 700, fontSize: "1.15rem", flexGrow: 1 }}>
          Notes
        </Typography>
      </Box>

      {/* Filter and Search Bar */}
      <Box sx={{ p: 2, pb: 1, display: "flex", flexDirection: "column", gap: 1.25 }}>
        <TextField
          size="small"
          placeholder="Search notes…"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <SearchRoundedIcon sx={{ fontSize: 20, color: "#808080" }} />
                </InputAdornment>
              ),
            },
          }}
          sx={{
            "& .MuiOutlinedInput-root": {
              borderRadius: 3,
              bgcolor: "#FAFAFA",
            },
          }}
        />

        <Box sx={{ display: "flex", gap: 0.75 }}>
          <Chip
            size="small"
            label="Active"
            color={!noteArchiveFilter ? "primary" : "default"}
            onClick={() => setNoteArchiveFilter(false)}
          />
          <Chip
            size="small"
            label="Archived"
            color={noteArchiveFilter ? "primary" : "default"}
            onClick={() => setNoteArchiveFilter(true)}
          />
        </Box>
      </Box>

      {/* Main Content Area */}
      <Box sx={{ flexGrow: 1, overflowY: "auto", px: 2, pb: 10, pt: 1 }}>
        {notes.length === 0 ? (
          <Typography color="text.secondary" sx={{ py: 6, textAlign: "center", fontSize: "0.9rem" }}>
            No notes found. Tap + to write a note.
          </Typography>
        ) : (
          notes.map((n) => (
            <Card
              key={n.id}
              variant="outlined"
              sx={{
                mb: 1.5,
                borderRadius: 2.5,
                borderColor: "#EEEEEE",
                cursor: "pointer",
                "&:hover": { bgcolor: "#FAFAFA" },
              }}
              onClick={() => {
                setEditingNote(n);
                setNoteTitle(n.title);
                setNoteContent(n.content);
                setNoteTagsInput(n.tags ? n.tags.join(", ") : "");
                setNoteModalOpen(true);
              }}
            >
              <CardContent sx={{ py: 1.5, px: 2, "&:last-child": { pb: 1.5 } }}>
                <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <Typography variant="subtitle1" sx={{ fontWeight: 600, fontSize: "0.95rem" }}>
                    {n.title}
                  </Typography>
                  <Box sx={{ display: "flex", gap: 0.5 }}>
                    <IconButton
                      size="small"
                      onClick={(e) => {
                        e.stopPropagation();
                        void handleToggleArchiveNote(n);
                      }}
                    >
                      {n.is_archived ? (
                        <UnarchiveOutlinedIcon sx={{ fontSize: 18 }} />
                      ) : (
                        <ArchiveOutlinedIcon sx={{ fontSize: 18 }} />
                      )}
                    </IconButton>
                    <IconButton
                      size="small"
                      onClick={(e) => {
                        e.stopPropagation();
                        void handleDeleteNote(n.id);
                      }}
                    >
                      <DeleteOutlineRoundedIcon sx={{ fontSize: 18, color: "#808080" }} />
                    </IconButton>
                  </Box>
                </Box>
                <Typography
                  variant="body2"
                  color="text.secondary"
                  sx={{
                    mt: 0.5,
                    fontSize: "0.85rem",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    display: "-webkit-box",
                    WebkitLineClamp: 3,
                    WebkitBoxOrient: "vertical",
                  }}
                >
                  {n.content}
                </Typography>
                {n.tags && n.tags.length > 0 && (
                  <Box sx={{ display: "flex", gap: 0.5, mt: 1, flexWrap: "wrap" }}>
                    {n.tags.map((tag) => (
                      <Chip
                        key={tag}
                        size="small"
                        label={`#${tag}`}
                        variant="outlined"
                        sx={{ fontSize: "0.7rem", height: 20 }}
                      />
                    ))}
                  </Box>
                )}
              </CardContent>
            </Card>
          ))
        )}
      </Box>

      {/* Floating Action Button */}
      <Fab
        color="primary"
        aria-label="Add note"
        onClick={() => {
          setEditingNote(null);
          setNoteTitle("");
          setNoteContent("");
          setNoteTagsInput("");
          setNoteModalOpen(true);
        }}
        sx={{
          position: "fixed",
          right: 20,
          bottom: `calc(24px + env(safe-area-inset-bottom))`,
          bgcolor: "#DC4C3E",
          "&:hover": { bgcolor: "#B9382B" },
        }}
      >
        <AddRoundedIcon />
      </Fab>

      {/* Note Modal */}
      <Dialog
        open={noteModalOpen}
        onClose={() => setNoteModalOpen(false)}
        fullWidth
        maxWidth="xs"
      >
        <DialogTitle sx={{ fontWeight: 700 }}>
          {editingNote ? "Edit Note" : "New Note"}
        </DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            autoFocus
            label="Note Title"
            value={noteTitle}
            onChange={(e) => setNoteTitle(e.target.value)}
            fullWidth
            required
          />
          <TextField
            label="Content"
            value={noteContent}
            onChange={(e) => setNoteContent(e.target.value)}
            fullWidth
            multiline
            rows={4}
          />
          <TextField
            label="Tags (comma separated)"
            value={noteTagsInput}
            onChange={(e) => setNoteTagsInput(e.target.value)}
            placeholder="work, personal, idea"
            fullWidth
            size="small"
          />
        </DialogContent>
        <DialogActions sx={{ p: 2 }}>
          <Button onClick={() => setNoteModalOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => void handleSaveNote()}
            disabled={!noteTitle.trim()}
            sx={{ bgcolor: "#DC4C3E", "&:hover": { bgcolor: "#B9382B" } }}
          >
            Save Note
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}
