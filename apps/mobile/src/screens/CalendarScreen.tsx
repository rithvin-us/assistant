/**

 * Lightweight Google Calendar & Schedule Screen.

 *

 * Implements:

 * - Multi-account selection (Personal, College, Work)

 * - Today, Upcoming, and Agenda views

 * - Deterministic Free Time finder (zero AI, interval calculation)

 * - Create & delete calendar events

 * - Pure Light Theme aesthetics

 */



import { useState, useEffect, useCallback, useMemo } from "react";

import Box from "@mui/material/Box";

import Typography from "@mui/material/Typography";

import Button from "@mui/material/Button";

import Chip from "@mui/material/Chip";

import IconButton from "@mui/material/IconButton";

import TextField from "@mui/material/TextField";

import Dialog from "@mui/material/Dialog";

import DialogTitle from "@mui/material/DialogTitle";

import DialogContent from "@mui/material/DialogContent";

import DialogActions from "@mui/material/DialogActions";

import CircularProgress from "@mui/material/CircularProgress";

import Card from "@mui/material/Card";

import CardContent from "@mui/material/CardContent";

import Tabs from "@mui/material/Tabs";

import Tab from "@mui/material/Tab";

import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";

import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";

import AddRoundedIcon from "@mui/icons-material/AddRounded";

import DeleteOutlineRoundedIcon from "@mui/icons-material/DeleteOutlineRounded";

import EventRoundedIcon from "@mui/icons-material/EventRounded";

import AccessTimeRoundedIcon from "@mui/icons-material/AccessTimeRounded";

import LocationOnOutlinedIcon from "@mui/icons-material/LocationOnOutlined";

import ScheduleRoundedIcon from "@mui/icons-material/ScheduleRounded";



import type { AccountSummary, CalendarEvent, FreeSlot } from "../api/types";

import {

  fetchGoogleAccounts,

  fetchCalendarEvents,

  createCalendarEvent,

  deleteCalendarEvent,

  fetchFreeSlots,

} from "../api/google";



interface CalendarScreenProps {

  onBack?: () => void;

}



export default function CalendarScreen({ onBack }: CalendarScreenProps) {

  const [accounts, setAccounts] = useState<AccountSummary[]>([]);

  const [selectedAccountId, setSelectedAccountId] = useState<string>("");

  const [events, setEvents] = useState<CalendarEvent[]>([]);

  const [loading, setLoading] = useState(true);

  const [viewTab, setViewTab] = useState<"today" | "upcoming" | "freetime">("today");



  // Create Event Modal

  const [createModalOpen, setCreateModalOpen] = useState(false);

  const [newTitle, setNewTitle] = useState("");

  const [newDate, setNewDate] = useState(() => new Date().toISOString().slice(0, 10));

  const [newStartTime, setNewStartTime] = useState("10:00");

  const [newEndTime, setNewEndTime] = useState("11:00");

  const [newLocation, setNewLocation] = useState("");

  const [newDescription, setNewDescription] = useState("");

  const [savingEvent, setSavingEvent] = useState(false);



  // Free Time Engine

  const [freeDuration, setFreeDuration] = useState<number>(60);

  const [freeSlots, setFreeSlots] = useState<FreeSlot[]>([]);

  const [loadingSlots, setLoadingSlots] = useState(false);



  // Delete Target

  const [deleteTarget, setDeleteTarget] = useState<CalendarEvent | null>(null);

  const [errorMessage, setErrorMessage] = useState<string | null>(null);



  // Load Accounts

  useEffect(() => {

    async function init() {

      try {

        const accs = await fetchGoogleAccounts();

        setAccounts(accs);

        if (accs.length > 0 && !selectedAccountId) {

          setSelectedAccountId(accs[0].id);

        }

      } catch (err: unknown) {

        const msg = err instanceof Error ? err.message : "Failed to load accounts";

        setErrorMessage(msg);

      }

    }

    init();

  }, [selectedAccountId]);



  // Load Events for Selected Account

  const loadEvents = useCallback(async () => {

    if (!selectedAccountId) {

      setEvents([]);

      setLoading(false);

      return;

    }

    try {

      setLoading(true);

      setErrorMessage(null);

      const now = new Date();

      const minDate = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1).toISOString();

      const maxDate = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 14).toISOString();

      const list = await fetchCalendarEvents(selectedAccountId, minDate, maxDate);

      setEvents(list);

    } catch (err: unknown) {

      const msg = err instanceof Error ? err.message : "Failed to load calendar events";

      setErrorMessage(msg);

    } finally {

      setLoading(false);

    }

  }, [selectedAccountId]);



  useEffect(() => {

    // eslint-disable-next-line react-hooks/set-state-in-effect
    loadEvents();

  }, [loadEvents]);



  // Find Free Slots

  const handleFindFreeSlots = useCallback(async () => {

    if (!selectedAccountId) return;

    try {

      setLoadingSlots(true);

      const now = new Date();

      const startOfDay = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 9, 0, 0).toISOString();

      const endOfDay = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 21, 0, 0).toISOString();

      const slots = await fetchFreeSlots(selectedAccountId, startOfDay, endOfDay, freeDuration);

      setFreeSlots(slots);

    } catch (err: unknown) {

      const msg = err instanceof Error ? err.message : "Failed to calculate free time";

      setErrorMessage(msg);

    } finally {

      setLoadingSlots(false);

    }

  }, [selectedAccountId, freeDuration]);



  useEffect(() => {

    if (viewTab === "freetime") {

      // eslint-disable-next-line react-hooks/set-state-in-effect
      handleFindFreeSlots();

    }

  }, [viewTab, handleFindFreeSlots]);



  const handleCreateEvent = async () => {

    if (!selectedAccountId || !newTitle.trim()) return;

    try {

      setSavingEvent(true);

      const startIso = `${newDate}T${newStartTime}:00Z`;

      const endIso = `${newDate}T${newEndTime}:00Z`;

      await createCalendarEvent(selectedAccountId, {

        account_id: selectedAccountId,

        title: newTitle.trim(),

        start_time: startIso,

        end_time: endIso,

        location: newLocation.trim() || undefined,

        description: newDescription.trim() || undefined,

      });

      setCreateModalOpen(false);

      setNewTitle("");

      setNewLocation("");

      setNewDescription("");

      await loadEvents();

    } catch (err: unknown) {

      const msg = err instanceof Error ? err.message : "Failed to create event";

      setErrorMessage(msg);

    } finally {

      setSavingEvent(false);

    }

  };



  const handleDeleteConfirm = async () => {

    if (!selectedAccountId || !deleteTarget) return;

    try {

      await deleteCalendarEvent(selectedAccountId, deleteTarget.id);

      setDeleteTarget(null);

      await loadEvents();

    } catch (err: unknown) {

      const msg = err instanceof Error ? err.message : "Failed to delete event";

      setErrorMessage(msg);

    }

  };



  const filteredEvents = useMemo(() => {

    const todayStr = new Date().toISOString().slice(0, 10);

    if (viewTab === "today") {

      return events.filter((e) => e.start_time.startsWith(todayStr));

    }

    if (viewTab === "upcoming") {

      return events.filter((e) => !e.start_time.startsWith(todayStr));

    }

    return events;

  }, [events, viewTab]);



  const formatTimeRange = (startIso: string, endIso: string) => {

    try {

      const s = new Date(startIso);

      const e = new Date(endIso);

      const sTime = s.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

      const eTime = e.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

      return `${sTime} – ${eTime}`;

    } catch {

      return `${startIso.slice(11, 16)} – ${endIso.slice(11, 16)}`;

    }

  };



  const formatDateLabel = (iso: string) => {

    try {

      const d = new Date(iso);

      return d.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });

    } catch {

      return iso.slice(0, 10);

    }

  };



  return (

    <Box

      sx={{

        display: "flex",

        flexDirection: "column",

        height: "100%",

        bgcolor: "#FFFFFF",

      }}

    >

      {/* Header */}

      <Box

        sx={{

          display: "flex",

          alignItems: "center",

          justifyContent: "space-between",

          px: 2,

          py: 1.5,

          borderBottom: "1px solid #F0F0F0",

        }}

      >

        <Box sx={{ display: "flex", alignItems: "center", gap: 1.5 }}>

          {onBack && (

            <IconButton onClick={onBack} size="small" edge="start" sx={{ color: "#202020" }}>

              <ArrowBackRoundedIcon />

            </IconButton>

          )}

          <Box>

            <Typography variant="h6" sx={{ fontWeight: 700, color: "#202020", fontSize: "1.15rem", lineHeight: 1.2 }}>

              Calendar

            </Typography>

            <Typography variant="caption" sx={{ color: "#808080" }}>

              Schedule & available slots

            </Typography>

          </Box>

        </Box>

        <Box sx={{ display: "flex", gap: 0.5 }}>

          <IconButton onClick={loadEvents} size="small" sx={{ color: "#606060" }}>

            <RefreshRoundedIcon />

          </IconButton>

          <IconButton

            onClick={() => setCreateModalOpen(true)}

            size="small"

            sx={{ bgcolor: "#202020", color: "#FFFFFF", "&:hover": { bgcolor: "#333333" } }}

          >

            <AddRoundedIcon fontSize="small" />

          </IconButton>

        </Box>

      </Box>



      {/* Account Selector Bar */}

      {accounts.length > 0 && (

        <Box sx={{ px: 2, pt: 1.5, pb: 0.5, display: "flex", alignItems: "center", gap: 1, overflowX: "auto" }}>

          <Typography variant="caption" sx={{ color: "#888888", fontWeight: 600, flexShrink: 0 }}>

            Account:

          </Typography>

          {accounts.map((acc) => {

            const isSelected = acc.id === selectedAccountId;

            return (

              <Chip

                key={acc.id}

                label={acc.email}

                clickable

                onClick={() => setSelectedAccountId(acc.id)}

                size="small"

                sx={{

                  fontWeight: isSelected ? 700 : 500,

                  bgcolor: isSelected ? "#202020" : "#F4F4F4",

                  color: isSelected ? "#FFFFFF" : "#505050",

                  "&:hover": { bgcolor: isSelected ? "#333333" : "#EAEAEA" },

                }}

              />

            );

          })}

        </Box>

      )}



      {/* View Tabs */}

      <Box sx={{ borderBottom: "1px solid #F0F0F0", px: 2 }}>

        <Tabs

          value={viewTab}

          onChange={(_, v) => setViewTab(v)}

          textColor="inherit"

          sx={{ minHeight: 42, "& .MuiTabs-indicator": { bgcolor: "#DC4C3E" } }}

        >

          <Tab value="today" label="Today" sx={{ textTransform: "none", fontWeight: 600, minHeight: 42, fontSize: "0.85rem" }} />

          <Tab value="upcoming" label="Upcoming" sx={{ textTransform: "none", fontWeight: 600, minHeight: 42, fontSize: "0.85rem" }} />

          <Tab value="freetime" label="Free Time" sx={{ textTransform: "none", fontWeight: 600, minHeight: 42, fontSize: "0.85rem" }} />

        </Tabs>

      </Box>



      {/* Content */}

      <Box sx={{ flex: 1, overflowY: "auto", p: 2 }}>

        {errorMessage && (

          <Box

            sx={{

              p: 1.5,

              mb: 2,

              borderRadius: 2,

              bgcolor: "#FFF4F2",

              border: "1px solid #FFEBE8",

              color: "#DC4C3E",

              fontSize: "0.85rem",

            }}

          >

            {errorMessage}

          </Box>

        )}



        {accounts.length === 0 ? (

          <Box sx={{ textAlign: "center", py: 6, color: "#808080" }}>

            <EventRoundedIcon sx={{ fontSize: 44, color: "#CCCCCC", mb: 1 }} />

            <Typography variant="body2">No connected Google accounts found.</Typography>

            <Typography variant="caption" sx={{ color: "#AAAAAA" }}>

              Go to Connected Accounts to connect Google Calendar.

            </Typography>

          </Box>

        ) : viewTab === "freetime" ? (

          /* Deterministic Free Time Engine View */

          <Box>

            <Card variant="outlined" sx={{ borderRadius: 2.5, borderColor: "#EBEBEB", bgcolor: "#FAFAFA", mb: 2.5 }}>

              <CardContent sx={{ p: 2, "&:last-child": { pb: 2 } }}>

                <Box sx={{ display: "flex", alignItems: "center", gap: 1, mb: 1 }}>

                  <ScheduleRoundedIcon sx={{ color: "#1A73E8", fontSize: 20 }} />

                  <Typography variant="subtitle2" sx={{ fontWeight: 700, color: "#202020" }}>

                    Deterministic Free-Time Finder

                  </Typography>

                </Box>

                <Typography variant="body2" sx={{ color: "#666666", fontSize: "0.82rem", mb: 2 }}>

                  Calculates exact free intervals today between 9:00 AM and 9:00 PM without requiring AI.

                </Typography>



                <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>

                  <Typography variant="caption" sx={{ fontWeight: 600, color: "#555555" }}>

                    Slot Duration:

                  </Typography>

                  {[30, 60, 90, 120].map((mins) => (

                    <Chip

                      key={mins}

                      label={`${mins}m`}

                      size="small"

                      clickable

                      onClick={() => setFreeDuration(mins)}

                      sx={{

                        fontWeight: freeDuration === mins ? 700 : 500,

                        bgcolor: freeDuration === mins ? "#1A73E8" : "#EFEFEF",

                        color: freeDuration === mins ? "#FFFFFF" : "#404040",

                      }}

                    />

                  ))}

                </Box>

              </CardContent>

            </Card>



            <Typography variant="overline" sx={{ color: "#888888", fontWeight: 700, letterSpacing: 0.8 }}>

              Available Slots ({freeSlots.length})

            </Typography>



            {loadingSlots ? (

              <Box sx={{ display: "flex", justifyContent: "center", py: 4 }}>

                <CircularProgress size={24} sx={{ color: "#1A73E8" }} />

              </Box>

            ) : freeSlots.length === 0 ? (

              <Box sx={{ textAlign: "center", py: 4, color: "#808080" }}>

                <Typography variant="body2">No available {freeDuration}-minute slots found today.</Typography>

              </Box>

            ) : (

              <Box sx={{ display: "flex", flexDirection: "column", gap: 1.5, mt: 1 }}>

                {freeSlots.map((slot, idx) => (

                  <Card

                    key={idx}

                    variant="outlined"

                    sx={{

                      borderRadius: 2,

                      borderColor: "#E2EDFC",

                      bgcolor: "#F6F9FE",

                    }}

                  >

                    <CardContent sx={{ p: 1.75, "&:last-child": { pb: 1.75 }, display: "flex", alignItems: "center", justifyContent: "space-between" }}>

                      <Box sx={{ display: "flex", alignItems: "center", gap: 1.5 }}>

                        <AccessTimeRoundedIcon sx={{ color: "#1A73E8", fontSize: 20 }} />

                        <Box>

                          <Typography variant="subtitle2" sx={{ fontWeight: 700, color: "#202020", lineHeight: 1.2 }}>

                            {formatTimeRange(slot.start_time, slot.end_time)}

                          </Typography>

                          <Typography variant="caption" sx={{ color: "#666666" }}>

                            {slot.duration_minutes} minutes available

                          </Typography>

                        </Box>

                      </Box>

                      <Button

                        size="small"

                        variant="outlined"

                        onClick={() => {

                          const s = new Date(slot.start_time);

                          const e = new Date(slot.end_time);

                          setNewDate(s.toISOString().slice(0, 10));

                          setNewStartTime(s.toTimeString().slice(0, 5));

                          setNewEndTime(e.toTimeString().slice(0, 5));

                          setCreateModalOpen(true);

                        }}

                        sx={{

                          borderColor: "#1A73E8",

                          color: "#1A73E8",

                          textTransform: "none",

                          fontWeight: 600,

                          borderRadius: 1.5,

                          fontSize: "0.75rem",

                          py: 0.5,

                        }}

                      >

                        Schedule

                      </Button>

                    </CardContent>

                  </Card>

                ))}

              </Box>

            )}

          </Box>

        ) : loading ? (

          <Box sx={{ display: "flex", justifyContent: "center", py: 5 }}>

            <CircularProgress size={24} sx={{ color: "#DC4C3E" }} />

          </Box>

        ) : filteredEvents.length === 0 ? (

          <Box sx={{ textAlign: "center", py: 5, color: "#808080" }}>

            <Typography variant="body2">No events found for this view.</Typography>

          </Box>

        ) : (

          /* Events List */

          <Box sx={{ display: "flex", flexDirection: "column", gap: 1.5 }}>

            {filteredEvents.map((evt) => (

              <Card

                key={evt.id}

                variant="outlined"

                sx={{

                  borderRadius: 2.5,

                  borderColor: "#EBEBEB",

                  bgcolor: "#FFFFFF",

                  borderLeft: "4px solid #1A73E8",

                }}

              >

                <CardContent sx={{ p: 2, "&:last-child": { pb: 2 } }}>

                  <Box sx={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between" }}>

                    <Box sx={{ flex: 1 }}>

                      <Typography variant="subtitle2" sx={{ fontWeight: 700, color: "#202020", lineHeight: 1.3 }}>

                        {evt.title}

                      </Typography>

                      <Box sx={{ display: "flex", alignItems: "center", gap: 0.75, mt: 0.75 }}>

                        <AccessTimeRoundedIcon sx={{ fontSize: 15, color: "#777777" }} />

                        <Typography variant="caption" sx={{ color: "#555555", fontWeight: 500 }}>

                          {formatDateLabel(evt.start_time)} • {formatTimeRange(evt.start_time, evt.end_time)}

                        </Typography>

                      </Box>

                      {evt.location && (

                        <Box sx={{ display: "flex", alignItems: "center", gap: 0.75, mt: 0.5 }}>

                          <LocationOnOutlinedIcon sx={{ fontSize: 15, color: "#777777" }} />

                          <Typography variant="caption" sx={{ color: "#666666" }}>

                            {evt.location}

                          </Typography>

                        </Box>

                      )}

                    </Box>

                    <IconButton

                      size="small"

                      onClick={() => setDeleteTarget(evt)}

                      sx={{ color: "#999999", "&:hover": { color: "#DC4C3E" } }}

                    >

                      <DeleteOutlineRoundedIcon fontSize="small" />

                    </IconButton>

                  </Box>

                </CardContent>

              </Card>

            ))}

          </Box>

        )}

      </Box>



      {/* Create Event Dialog */}

      <Dialog

        open={createModalOpen}

        onClose={() => setCreateModalOpen(false)}

        fullWidth

        maxWidth="xs"

        slotProps={{ paper: { sx: { borderRadius: 3, p: 1 } } }}

      >

        <DialogTitle sx={{ fontWeight: 700, color: "#202020" }}>

          New Calendar Event

        </DialogTitle>

        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: "8px !important" }}>

          <TextField

            autoFocus

            label="Event Title"

            fullWidth

            size="small"

            value={newTitle}

            onChange={(e) => setNewTitle(e.target.value)}

          />

          <TextField

            label="Date"

            type="date"

            fullWidth

            size="small"

            value={newDate}

            onChange={(e) => setNewDate(e.target.value)}

            slotProps={{ inputLabel: { shrink: true } }}

          />

          <Box sx={{ display: "flex", gap: 1.5 }}>

            <TextField

              label="Start Time"

              type="time"

              fullWidth

              size="small"

              value={newStartTime}

              onChange={(e) => setNewStartTime(e.target.value)}

              slotProps={{ inputLabel: { shrink: true } }}

            />

            <TextField

              label="End Time"

              type="time"

              fullWidth

              size="small"

              value={newEndTime}

              onChange={(e) => setNewEndTime(e.target.value)}

              slotProps={{ inputLabel: { shrink: true } }}

            />

          </Box>

          <TextField

            label="Location (Optional)"

            fullWidth

            size="small"

            value={newLocation}

            onChange={(e) => setNewLocation(e.target.value)}

          />

          <TextField

            label="Description (Optional)"

            fullWidth

            multiline

            rows={2}

            size="small"

            value={newDescription}

            onChange={(e) => setNewDescription(e.target.value)}

          />

        </DialogContent>

        <DialogActions sx={{ px: 2, pb: 2 }}>

          <Button onClick={() => setCreateModalOpen(false)} sx={{ color: "#666666", textTransform: "none", fontWeight: 600 }}>

            Cancel

          </Button>

          <Button

            onClick={handleCreateEvent}

            disabled={savingEvent || !newTitle.trim()}

            variant="contained"

            disableElevation

            sx={{

              bgcolor: "#202020",

              color: "#FFFFFF",

              textTransform: "none",

              fontWeight: 600,

              borderRadius: 2,

              "&:hover": { bgcolor: "#333333" },

            }}

          >

            {savingEvent ? "Saving..." : "Create Event"}

          </Button>

        </DialogActions>

      </Dialog>



      {/* Delete Confirmation Dialog */}

      <Dialog

        open={Boolean(deleteTarget)}

        onClose={() => setDeleteTarget(null)}

        slotProps={{ paper: { sx: { borderRadius: 3, p: 1 } } }}

      >

        <DialogTitle sx={{ fontWeight: 700, color: "#202020", pb: 1 }}>

          Delete Calendar Event?

        </DialogTitle>

        <DialogContent>

          <Typography variant="body2" sx={{ color: "#555555" }}>

            Are you sure you want to delete <strong>{deleteTarget?.title}</strong>? This action removes the event from Google Calendar.

          </Typography>

        </DialogContent>

        <DialogActions sx={{ pt: 1, px: 2, pb: 1.5 }}>

          <Button onClick={() => setDeleteTarget(null)} sx={{ color: "#666666", textTransform: "none", fontWeight: 600 }}>

            Cancel

          </Button>

          <Button

            onClick={handleDeleteConfirm}

            variant="contained"

            disableElevation

            sx={{

              bgcolor: "#DC4C3E",

              color: "#FFFFFF",

              textTransform: "none",

              fontWeight: 600,

              borderRadius: 2,

              "&:hover": { bgcolor: "#B83A2E" },

            }}

          >

            Delete

          </Button>

        </DialogActions>

      </Dialog>

    </Box>

  );

}

