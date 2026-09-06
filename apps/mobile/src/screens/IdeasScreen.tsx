/**
 * Standalone Ideas Page.
 *
 * Implements Ideas management & Idea-to-Task conversion with Todoist-inspired design.
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
import CardActions from "@mui/material/CardActions";
import IconButton from "@mui/material/IconButton";
import Fab from "@mui/material/Fab";
import SearchRoundedIcon from "@mui/icons-material/SearchRounded";
import AddRoundedIcon from "@mui/icons-material/AddRounded";
import DeleteOutlineRoundedIcon from "@mui/icons-material/DeleteOutlineRounded";
import TransformRoundedIcon from "@mui/icons-material/TransformRounded";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";

import type { IdeaItem } from "../api/types";
import {
  fetchIdeas,
  createIdea,
  updateIdea,
  convertIdeaToTask,
  deleteIdea,
} from "../api/productivity";

interface IdeasScreenProps {
  onBack?: () => void;
}

export default function IdeasScreen({ onBack }: IdeasScreenProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [ideas, setIdeas] = useState<IdeaItem[]>([]);
  const [ideaStatusFilter, setIdeaStatusFilter] = useState<string>("active");

  const [ideaModalOpen, setIdeaModalOpen] = useState(false);
  const [editingIdea, setEditingIdea] = useState<IdeaItem | null>(null);
  const [ideaTitle, setIdeaTitle] = useState("");
  const [ideaDesc, setIdeaDesc] = useState("");

  const loadData = async () => {
    try {
      const res = await fetchIdeas({
        status: ideaStatusFilter === "ALL" ? undefined : ideaStatusFilter,
        q: searchQuery || undefined,
      });
      setIdeas(res);
    } catch (err) {
      console.warn("Failed to load ideas:", err);
    }
  };

  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) return;
      void (async () => {
        try {
          const res = await fetchIdeas({
            status: ideaStatusFilter === "ALL" ? undefined : ideaStatusFilter,
            q: searchQuery || undefined,
          });
          if (!cancelled) setIdeas(res);
        } catch (err) {
          console.warn("Failed to load ideas:", err);
        }
      })();
    });
    return () => {
      cancelled = true;
    };
  }, [ideaStatusFilter, searchQuery]);

  const handleSaveIdea = async () => {
    if (!ideaTitle.trim()) return;
    try {
      if (editingIdea) {
        await updateIdea(editingIdea.id, {
          title: ideaTitle,
          description: ideaDesc,
        });
      } else {
        await createIdea({
          title: ideaTitle,
          description: ideaDesc,
        });
      }
      setIdeaModalOpen(false);
      setEditingIdea(null);
      setIdeaTitle("");
      setIdeaDesc("");
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleConvertIdea = async (idea: IdeaItem) => {
    try {
      await convertIdeaToTask(idea.id);
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleDeleteIdea = async (id: string) => {
    try {
      setIdeas((prev) => prev.filter((i) => i.id !== id));
      await deleteIdea(id);
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
      {/* Header Bar */}
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
          Ideas
        </Typography>
      </Box>

      {/* Filter and Search Bar */}
      <Box sx={{ p: 2, pb: 1, display: "flex", flexDirection: "column", gap: 1.25 }}>
        <TextField
          size="small"
          placeholder="Search ideas…"
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
          {["active", "converted", "ALL"].map((st) => (
            <Chip
              key={st}
              size="small"
              label={st === "active" ? "Active Ideas" : st === "converted" ? "Converted to Tasks" : "All"}
              color={ideaStatusFilter === st ? "primary" : "default"}
              onClick={() => setIdeaStatusFilter(st)}
            />
          ))}
        </Box>
      </Box>

      {/* Main Content Area */}
      <Box sx={{ flexGrow: 1, overflowY: "auto", px: 2, pb: 10, pt: 1 }}>
        {ideas.length === 0 ? (
          <Typography color="text.secondary" sx={{ py: 6, textAlign: "center", fontSize: "0.9rem" }}>
            No ideas captured. Tap + to record an idea.
          </Typography>
        ) : (
          ideas.map((idea) => (
            <Card key={idea.id} variant="outlined" sx={{ mb: 1.5, borderRadius: 2.5, borderColor: "#EEEEEE" }}>
              <CardContent sx={{ py: 1.5, px: 2, "&:last-child": { pb: 1 } }}>
                <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
                  <Typography variant="subtitle1" sx={{ fontWeight: 600, fontSize: "0.95rem" }}>
                    {idea.title}
                  </Typography>
                  <IconButton size="small" onClick={() => void handleDeleteIdea(idea.id)}>
                    <DeleteOutlineRoundedIcon sx={{ fontSize: 18, color: "#808080" }} />
                  </IconButton>
                </Box>
                {idea.description && (
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5, fontSize: "0.85rem" }}>
                    {idea.description}
                  </Typography>
                )}
              </CardContent>
              <CardActions sx={{ px: 2, pb: 1.5, pt: 0, justifyContent: "flex-end" }}>
                {idea.status === "active" ? (
                  <Button
                    size="small"
                    variant="outlined"
                    startIcon={<TransformRoundedIcon />}
                    onClick={() => void handleConvertIdea(idea)}
                    sx={{ textTransform: "none", fontSize: "0.78rem" }}
                  >
                    Convert to Task
                  </Button>
                ) : (
                  <Chip size="small" label="Converted to Task" color="success" variant="outlined" />
                )}
              </CardActions>
            </Card>
          ))
        )}
      </Box>

      {/* Floating Action Button */}
      <Fab
        color="primary"
        aria-label="Add idea"
        onClick={() => {
          setEditingIdea(null);
          setIdeaTitle("");
          setIdeaDesc("");
          setIdeaModalOpen(true);
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

      {/* Idea Modal */}
      <Dialog
        open={ideaModalOpen}
        onClose={() => setIdeaModalOpen(false)}
        fullWidth
        maxWidth="xs"
      >
        <DialogTitle sx={{ fontWeight: 700 }}>
          {editingIdea ? "Edit Idea" : "New Idea"}
        </DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            autoFocus
            label="Idea Title"
            value={ideaTitle}
            onChange={(e) => setIdeaTitle(e.target.value)}
            fullWidth
            required
          />
          <TextField
            label="Description (optional)"
            value={ideaDesc}
            onChange={(e) => setIdeaDesc(e.target.value)}
            fullWidth
            multiline
            rows={3}
          />
        </DialogContent>
        <DialogActions sx={{ p: 2 }}>
          <Button onClick={() => setIdeaModalOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => void handleSaveIdea()}
            disabled={!ideaTitle.trim()}
            sx={{ bgcolor: "#DC4C3E", "&:hover": { bgcolor: "#B9382B" } }}
          >
            Save Idea
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}
