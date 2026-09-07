/**
 * Classroom — courses, then one course's coursework and announcements.
 *
 * Reads the server's cache by default so the screen opens instantly and still
 * works with no network. "Refresh" is the only thing that calls Google, and it
 * is always something the user pressed.
 *
 * No AI is involved. A due date shown here came from Classroom as a date and a
 * time; one that is missing is shown as missing.
 */

import { useCallback, useEffect, useState } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import IconButton from "@mui/material/IconButton";
import List from "@mui/material/List";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemText from "@mui/material/ListItemText";
import Divider from "@mui/material/Divider";
import Chip from "@mui/material/Chip";
import CircularProgress from "@mui/material/CircularProgress";
import Alert from "@mui/material/Alert";
import Tabs from "@mui/material/Tabs";
import Tab from "@mui/material/Tab";
import Button from "@mui/material/Button";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";

import { fetchGoogleAccounts } from "../api/google";
import {
  fetchAnnouncements,
  fetchCourses,
  fetchCoursework,
  formatDue,
  formatSynced,
  syncAcademic,
} from "../api/academic";
import AccountPicker from "../components/AccountPicker";
import PullToRefresh from "../components/PullToRefresh";
import { hasScopesFor } from "../api/scopes";
import type {
  AccountSummary,
  Announcement,
  Course,
  CourseworkItem,
} from "../api/types";

interface Props {
  onBack: () => void;
  onOpenConnections?: () => void;
}

export default function ClassroomScreen({ onBack, onOpenConnections }: Props) {
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [accountId, setAccountId] = useState<string | null>(null);
  const [courses, setCourses] = useState<Course[]>([]);
  const [selectedCourse, setSelectedCourse] = useState<Course | null>(null);
  const [coursework, setCoursework] = useState<CourseworkItem[]>([]);
  const [announcements, setAnnouncements] = useState<Announcement[]>([]);
  const [tab, setTab] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

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

  const loadCourses = useCallback(async (id: string) => {
    try {
      const next = await fetchCourses(id);
      setCourses(next);
      setError(null);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    if (!accountId) return;
    let cancelled = false;
    void (async () => {
      if (cancelled) return;
      await loadCourses(accountId);
    })();
    return () => {
      cancelled = true;
    };
  }, [accountId, loadCourses]);

  const openCourse = useCallback(
    async (course: Course) => {
      if (!accountId) return;
      setSelectedCourse(course);
      setTab(0);
      setBusy(true);
      setError(null);
      try {
        const [work, notes] = await Promise.all([
          fetchCoursework(accountId, course.external_id),
          fetchAnnouncements(accountId, course.external_id),
        ]);
        setCoursework(work);
        setAnnouncements(notes);
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [accountId],
  );

  /**
   * The only path that calls Google. Runs a full sync so coursework also lands
   * in Tasks, then re-reads the cache.
   */
  const refresh = useCallback(async () => {
    if (!accountId) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const result = await syncAcademic(accountId);
      setNotice(
        `${result.courses_synced} courses, ${result.coursework_synced} assignments. ` +
          `${result.tasks_created} tasks added, ${result.tasks_updated} updated` +
          (result.tasks_skipped_user_edited > 0
            ? `, ${result.tasks_skipped_user_edited} left as you edited them.`
            : "."),
      );
      await loadCourses(accountId);
      if (selectedCourse) await openCourse(selectedCourse);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [accountId, loadCourses, openCourse, selectedCourse]);

  const handleRefresh = async () => {
    await refresh();
  };

  const selectedAccount = accounts.find((a) => a.id === accountId);
  const canUse = hasScopesFor(selectedAccount, "classroom");

  return (
    <Box sx={{ height: "100%", display: "flex", flexDirection: "column" }}>
      <Box sx={{ display: "flex", alignItems: "center", gap: 1, px: 1, pt: 1 }}>
        <IconButton
          onClick={() => (selectedCourse ? setSelectedCourse(null) : onBack())}
          aria-label="Back"
        >
          <ArrowBackRoundedIcon />
        </IconButton>
        <Typography variant="h6" sx={{ flex: 1, fontWeight: 700 }}>
          {selectedCourse ? selectedCourse.name : "Classroom"}
        </Typography>
        <IconButton onClick={() => void refresh()} disabled={busy || !canUse} aria-label="Refresh">
          <RefreshRoundedIcon />
        </IconButton>
      </Box>

      {!selectedCourse && (
        <AccountPicker
          accounts={accounts}
          selectedId={accountId}
          onSelect={setAccountId}
          feature="classroom"
          onOpenConnections={onOpenConnections}
        />
      )}

      {error && (
        <Alert severity="error" sx={{ mx: 2, mb: 1 }} onClose={() => setError(null)}>
          {error}
        </Alert>
      )}
      {notice && (
        <Alert severity="success" sx={{ mx: 2, mb: 1 }} onClose={() => setNotice(null)}>
          {notice}
        </Alert>
      )}

      {busy && (
        <Box sx={{ display: "flex", justifyContent: "center", py: 3 }}>
          <CircularProgress size={22} />
        </Box>
      )}

      <PullToRefresh onRefresh={handleRefresh} disabled={busy || !canUse || !accountId}>
        {!selectedCourse && !busy && courses.length === 0 && canUse && (
          <Box sx={{ px: 3, py: 4, textAlign: "center" }}>
            <Typography variant="body2" sx={{ color: "text.secondary", mb: 2 }}>
              Nothing cached yet for this account.
            </Typography>
            <Button variant="outlined" size="small" onClick={() => void refresh()}>
              Sync Classroom
            </Button>
          </Box>
        )}

        {!selectedCourse &&
          courses.map((course) => (
            <Box key={course.external_id}>
              <ListItemButton onClick={() => void openCourse(course)}>
                <ListItemText
                  primary={course.name}
                  secondary={
                    [course.section, course.room].filter(Boolean).join(" · ") ||
                    formatSynced(course.synced_at)
                  }
                  slotProps={{ primary: { sx: { fontWeight: 600 } } }}
                />
                {course.state !== "ACTIVE" && (
                  <Chip label={course.state.toLowerCase()} size="small" variant="outlined" />
                )}
              </ListItemButton>
              <Divider component="li" sx={{ listStyle: "none" }} />
            </Box>
          ))}

        {selectedCourse && (
          <>
            <Tabs value={tab} onChange={(_, v: number) => setTab(v)} variant="fullWidth">
              <Tab label={`Coursework (${coursework.length})`} />
              <Tab label={`Announcements (${announcements.length})`} />
            </Tabs>

            {tab === 0 && (
              <List disablePadding>
                {coursework.length === 0 && !busy && (
                  <Typography variant="body2" sx={{ px: 3, py: 3, color: "text.secondary" }}>
                    No coursework cached for this course.
                  </Typography>
                )}
                {coursework.map((item) => (
                  <Box key={item.external_id}>
                    <ListItemButton
                      component="a"
                      href={item.alternate_link ?? undefined}
                      target="_blank"
                      rel="noreferrer"
                    >
                      <ListItemText
                        primary={item.title}
                        secondary={formatDue(item.due_at)}
                        slotProps={{
                          primary: { sx: { fontWeight: 600 } },
                          secondary: {
                            // A missing deadline is stated, not hidden.
                            color: item.due_at ? "text.secondary" : "text.disabled",
                          },
                        }}
                      />
                      {item.materials.length > 0 && (
                        <Chip
                          label={`${item.materials.length} file${item.materials.length > 1 ? "s" : ""}`}
                          size="small"
                          variant="outlined"
                        />
                      )}
                    </ListItemButton>
                    <Divider component="li" sx={{ listStyle: "none" }} />
                  </Box>
                ))}
              </List>
            )}

            {tab === 1 && (
              <List disablePadding>
                {announcements.length === 0 && !busy && (
                  <Typography variant="body2" sx={{ px: 3, py: 3, color: "text.secondary" }}>
                    No announcements cached for this course.
                  </Typography>
                )}
                {announcements.map((a) => (
                  <Box key={a.external_id}>
                    <ListItemButton
                      component="a"
                      href={a.alternate_link ?? undefined}
                      target="_blank"
                      rel="noreferrer"
                    >
                      <ListItemText
                        primary={a.text || "(no text)"}
                        secondary={formatSynced(a.synced_at)}
                        slotProps={{
                          primary: {
                            sx: {
                              display: "-webkit-box",
                              WebkitLineClamp: 3,
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
              </List>
            )}
          </>
        )}
      </PullToRefresh>
    </Box>
  );
}
