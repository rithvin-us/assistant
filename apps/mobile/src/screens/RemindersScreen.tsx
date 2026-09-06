/**
 * Standalone Reminders Page.
 *
 * Implements Reminders management with Todoist-inspired visual language.
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
import AlarmRoundedIcon from "@mui/icons-material/AlarmRounded";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";

import type { ReminderItem } from "../api/types";
import {
  fetchReminders,
  createReminder,
  updateReminder,
  deleteReminder,
} from "../api/productivity";

interface RemindersScreenProps {
  onBack?: () => void;
}

export default function RemindersScreen({ onBack }: RemindersScreenProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [reminders, setReminders] = useState<ReminderItem[]>([]);
  const [reminderStatusFilter, setReminderStatusFilter] = useState<string>("pending");

  const [reminderModalOpen, setReminderModalOpen] = useState(false);
  const [editingReminder, setEditingReminder] = useState<ReminderItem | null>(null);
  const [reminderTitle, setReminderTitle] = useState("");
  const [reminderTime, setReminderTime] = useState("");

  const loadData = async () => {
    try {
      const res = await fetchReminders({
        status: reminderStatusFilter === "ALL" ? undefined : reminderStatusFilter,
        q: searchQuery || undefined,
      });
      setReminders(res);
    } catch (err) {
      console.warn("Failed to load reminders:", err);
    }
  };

  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) return;
      void (async () => {
        try {
          const res = await fetchReminders({
            status: reminderStatusFilter === "ALL" ? undefined : reminderStatusFilter,
            q: searchQuery || undefined,
          });
          if (!cancelled) setReminders(res);
        } catch (err) {
          console.warn("Failed to load reminders:", err);
        }
      })();
    });
    return () => {
      cancelled = true;
    };
  }, [reminderStatusFilter, searchQuery]);

  const handleSaveReminder = async () => {
    if (!reminderTitle.trim() || !reminderTime) return;
    try {
      const remindAtIso = new Date(reminderTime).toISOString();
      if (editingReminder) {
        await updateReminder(editingReminder.id, {
          title: reminderTitle,
          remind_at: remindAtIso,
        });
      } else {
        await createReminder({
          title: reminderTitle,
          remind_at: remindAtIso,
        });
      }
      setReminderModalOpen(false);
      setEditingReminder(null);
      setReminderTitle("");
      setReminderTime("");
      await loadData();
    } catch (err) {
      console.error(err);
    }
  };

  const handleToggleReminderHandled = async (reminder: ReminderItem) => {
    const nextStatus = reminder.status === "handled" ? "pending" : "handled";
    try {
      setReminders((prev) =>
        prev.map((r) => (r.id === reminder.id ? { ...r, status: nextStatus } : r))
      );
      await updateReminder(reminder.id, { status: nextStatus });
    } catch (err) {
      console.error(err);
    }
  };

  const handleDeleteReminder = async (id: string) => {
    try {
      setReminders((prev) => prev.filter((r) => r.id !== id));
      await deleteReminder(id);
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
      {/* Top Header */}
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
          Reminders
        </Typography>
      </Box>

      {/* Filter and Search Bar */}
      <Box sx={{ p: 2, pb: 1, display: "flex", flexDirection: "column", gap: 1.25 }}>
        <TextField
          size="small"
          placeholder="Search reminders…"
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
          {["pending", "handled", "ALL"].map((st) => (
            <Chip
              key={st}
              size="small"
              label={st === "pending" ? "Upcoming" : st === "handled" ? "Handled" : "All"}
              color={reminderStatusFilter === st ? "primary" : "default"}
              onClick={() => setReminderStatusFilter(st)}
              sx={{ fontWeight: 500 }}
            />
          ))}
        </Box>
      </Box>

      {/* Main Content Area */}
      <Box sx={{ flexGrow: 1, overflowY: "auto", px: 2, pb: 10, pt: 1 }}>
        {reminders.length === 0 ? (
          <Typography color="text.secondary" sx={{ py: 6, textAlign: "center", fontSize: "0.9rem" }}>
            No reminders scheduled. Tap + to set a reminder.
          </Typography>
        ) : (
          reminders.map((r) => (
            <Card key={r.id} variant="outlined" sx={{ mb: 1.5, borderRadius: 2.5, borderColor: "#EEEEEE" }}>
              <CardContent sx={{ py: 1.5, px: 2, "&:last-child": { pb: 1.5 } }}>
                <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
                  <Box>
                    <Typography
                      variant="subtitle1"
                      sx={{
                        fontWeight: 600,
                        fontSize: "0.95rem",
                        textDecoration: r.status === "handled" ? "line-through" : "none",
                        color: r.status === "handled" ? "text.secondary" : "text.primary",
                      }}
                    >
                      {r.title}
                    </Typography>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "primary.main",
                        fontWeight: 600,
                        display: "flex",
                        alignItems: "center",
                        gap: 0.5,
                        mt: 0.5,
                      }}
                    >
                      <AlarmRoundedIcon sx={{ fontSize: 14 }} />
                      {new Date(r.remind_at).toLocaleString()}
                    </Typography>
                  </Box>
                  <Box sx={{ display: "flex", alignItems: "center", gap: 0.5 }}>
                    <Button
                      size="small"
                      variant={r.status === "handled" ? "outlined" : "contained"}
                      onClick={() => void handleToggleReminderHandled(r)}
                      sx={{ textTransform: "none", fontSize: "0.75rem", px: 1.5, py: 0.25 }}
                    >
                      {r.status === "handled" ? "Reopen" : "Done"}
                    </Button>
                    <IconButton size="small" onClick={() => void handleDeleteReminder(r.id)}>
                      <DeleteOutlineRoundedIcon sx={{ fontSize: 18, color: "#808080" }} />
                    </IconButton>
                  </Box>
                </Box>
              </CardContent>
            </Card>
          ))
        )}
      </Box>

      {/* Floating Action Button */}
      <Fab
        color="primary"
        aria-label="Add reminder"
        onClick={() => {
          setEditingReminder(null);
          setReminderTitle("");
          const defaultTime = new Date(Date.now() + 3600 * 1000).toISOString().slice(0, 16);
          setReminderTime(defaultTime);
          setReminderModalOpen(true);
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

      {/* Reminder Modal */}
      <Dialog
        open={reminderModalOpen}
        onClose={() => setReminderModalOpen(false)}
        fullWidth
        maxWidth="xs"
      >
        <DialogTitle sx={{ fontWeight: 700 }}>
          {editingReminder ? "Edit Reminder" : "Add Reminder"}
        </DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            autoFocus
            label="Reminder Title"
            value={reminderTitle}
            onChange={(e) => setReminderTitle(e.target.value)}
            fullWidth
            required
          />
          <TextField
            type="datetime-local"
            label="Remind At"
            value={reminderTime}
            onChange={(e) => setReminderTime(e.target.value)}
            slotProps={{ inputLabel: { shrink: true } }}
            fullWidth
            required
          />
        </DialogContent>
        <DialogActions sx={{ p: 2 }}>
          <Button onClick={() => setReminderModalOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => void handleSaveReminder()}
            disabled={!reminderTitle.trim() || !reminderTime}
            sx={{ bgcolor: "#DC4C3E", "&:hover": { bgcolor: "#B9382B" } }}
          >
            Save Reminder
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}
