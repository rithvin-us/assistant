/**



 * Gmail Screen — Search & Read.



 *



 * Implements:



 * - Multi-account selection for strict account isolation



 * - Search with native Gmail query syntax (from:, to:, subject:, is:unread)



 * - Email summary feed (sender, subject, snippet, date, unread indicator)



 * - On-demand email reader drawer (privacy preserved, no DB mirror)



 * - Pure Light Theme aesthetics



 */







import { useState, useEffect, useCallback } from "react";



import Box from "@mui/material/Box";



import Typography from "@mui/material/Typography";



import TextField from "@mui/material/TextField";



import InputAdornment from "@mui/material/InputAdornment";





import Chip from "@mui/material/Chip";



import IconButton from "@mui/material/IconButton";



import Drawer from "@mui/material/Drawer";



import CircularProgress from "@mui/material/CircularProgress";



import Card from "@mui/material/Card";



import CardContent from "@mui/material/CardContent";



import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";



import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";



import SearchRoundedIcon from "@mui/icons-material/SearchRounded";



import MailOutlineRoundedIcon from "@mui/icons-material/MailOutlineRounded";



import CloseRoundedIcon from "@mui/icons-material/CloseRounded";







import type { AccountSummary, EmailDetail, EmailSummary } from "../api/types";



import { fetchGoogleAccounts, searchGmail, readGmail } from "../api/google";







interface GmailScreenProps {



  onBack?: () => void;



}







export default function GmailScreen({ onBack }: GmailScreenProps) {



  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [selectedAccountId, setSelectedAccountId] = useState<string>("");
  const [searchQuery, setSearchQuery] = useState("");
  const [emails, setEmails] = useState<EmailSummary[]>([]);
  const [loading, setLoading] = useState(false);

  // Selected Email for Detail View
  const [selectedEmailId, setSelectedEmailId] = useState<string | null>(null);
  const [emailDetail, setEmailDetail] = useState<EmailDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);

  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // Touch Swipe Down Handler State for Drawer
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
      setSelectedEmailId(null);
    }
    setTouchStartY(null);
    setDragOffsetY(0);
  };

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

  // Execute Search
  const handleSearch = useCallback(async (silent = false) => {
    if (!selectedAccountId) {
      setEmails([]);
      return;
    }

    try {
      if (!silent) setLoading(true);
      setErrorMessage(null);
      // Default to "in:inbox" if empty so normal mailbox is shown first
      const queryToRun = searchQuery.trim() ? searchQuery.trim() : "in:inbox";
      const results = await searchGmail(selectedAccountId, queryToRun, 25);
      setEmails(results);
    } catch (err: unknown) {
      if (!silent) {
        const msg = err instanceof Error ? err.message : "Gmail search failed";
        setErrorMessage(msg);
      }
    } finally {
      if (!silent) setLoading(false);
    }
  }, [selectedAccountId, searchQuery]);

  useEffect(() => {
    if (!selectedAccountId) return;
    let cancelled = false;

    void handleSearch(false);

    const interval = setInterval(() => {
      if (!cancelled) void handleSearch(true);
    }, 10000);

    const onFocus = () => {
      if (!cancelled) void handleSearch(true);
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);

    return () => {
      cancelled = true;
      clearInterval(interval);
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, [selectedAccountId, handleSearch]);

  // Open Email Detail
  const handleOpenEmail = async (email: EmailSummary) => {
    if (!selectedAccountId) return;
    setSelectedEmailId(email.id);
    setEmailDetail(null);
    setDragOffsetY(0);

    // Optimistically mark email as read in local state without removing it from list
    setEmails((prev) =>
      prev.map((e) => (e.id === email.id ? { ...e, is_unread: false } : e))
    );

    try {
      setLoadingDetail(true);
      const detail = await readGmail(selectedAccountId, email.id);
      setEmailDetail(detail);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Failed to read email";
      setErrorMessage(msg);
    } finally {
      setLoadingDetail(false);
    }
  };

  const formatDateLabel = (dateStr?: string | null) => {
    if (!dateStr) return "";
    try {
      const d = new Date(dateStr);
      return d.toLocaleDateString([], { month: "short", day: "numeric" });
    } catch {
      return dateStr.slice(0, 10);
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
              Gmail
            </Typography>
            <Typography variant="caption" sx={{ color: "#808080" }}>
              Inbox & mail search
            </Typography>
          </Box>
        </Box>
        <IconButton onClick={() => void handleSearch(false)} size="small" sx={{ color: "#606060" }}>
          <RefreshRoundedIcon />
        </IconButton>
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

      {/* Search Input Bar */}
      <Box sx={{ px: 2, py: 1.5 }}>
        <TextField
          fullWidth
          size="small"
          placeholder="Search mail (e.g. from:prof, subject:exam or leave blank for Inbox)"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleSearch();
          }}
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <SearchRoundedIcon fontSize="small" sx={{ color: "#808080" }} />
                </InputAdornment>
              ),
              sx: {
                borderRadius: 2.5,
                bgcolor: "#F7F7F7",
                fontSize: "0.85rem",
                "& fieldset": { borderColor: "#EBEBEB" },
              },
            },
          }}
        />

        {/* Quick Query Filters */}
        <Box sx={{ display: "flex", gap: 0.75, mt: 1, overflowX: "auto" }}>
          {[
            { label: "All Inbox", query: "" },
            { label: "Unread", query: "is:unread" },
            { label: "Starred", query: "is:starred" },
            { label: "Important", query: "is:important" },
            { label: "Today", query: "newer_than:1d" },
          ].map((item) => (
            <Chip
              key={item.label}
              label={item.label}
              size="small"
              clickable
              onClick={() => {
                setSearchQuery(item.query);
              }}
              sx={{
                fontSize: "0.72rem",
                fontWeight: searchQuery === item.query ? 700 : 500,
                bgcolor: searchQuery === item.query ? "#DC4C3E" : "#F0F0F0",
                color: searchQuery === item.query ? "#FFFFFF" : "#404040",
              }}
            />
          ))}
        </Box>
      </Box>

      {/* Main Email List */}
      <Box sx={{ flex: 1, overflowY: "auto", px: 2, pb: 2 }}>
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
            <MailOutlineRoundedIcon sx={{ fontSize: 44, color: "#CCCCCC", mb: 1 }} />
            <Typography variant="body2">No connected Google accounts found.</Typography>
            <Typography variant="caption" sx={{ color: "#AAAAAA" }}>
              Connect a Google account to read Gmail.
            </Typography>
          </Box>
        ) : loading ? (
          <Box sx={{ display: "flex", justifyContent: "center", py: 5 }}>
            <CircularProgress size={24} sx={{ color: "#DC4C3E" }} />
          </Box>
        ) : emails.length === 0 ? (
          <Box sx={{ textAlign: "center", py: 6, color: "#808080" }}>
            <Typography variant="body2">No emails found matching view.</Typography>
          </Box>
        ) : (
          <Box sx={{ display: "flex", flexDirection: "column", gap: 1 }}>
            {emails.map((m) => (
              <Card
                key={m.id}
                variant="outlined"
                onClick={() => handleOpenEmail(m)}
                sx={{
                  borderRadius: 2.5,
                  borderColor: m.is_unread ? "#DCE6F8" : "#F0F0F0",
                  bgcolor: m.is_unread ? "#FFFFFF" : "#FAFAFA",
                  cursor: "pointer",
                  transition: "all 0.15s ease",
                  boxShadow: m.is_unread ? "0 2px 8px rgba(26,115,232,0.06)" : "none",
                  "&:hover": { bgcolor: "#F5F5F5", borderColor: "#C0D4F5" },
                }}
              >
                <CardContent sx={{ p: 1.75, "&:last-child": { pb: 1.75 } }}>
                  <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", mb: 0.5 }}>
                    <Box sx={{ display: "flex", alignItems: "center", gap: 0.75, flex: 1, minWidth: 0 }}>
                      {m.is_unread && (
                        <Box sx={{ width: 8, height: 8, borderRadius: "50%", bgcolor: "#1A73E8", flexShrink: 0 }} />
                      )}
                      <Typography
                        variant="caption"
                        sx={{
                          fontWeight: m.is_unread ? 700 : 600,
                          color: m.is_unread ? "#111827" : "#4B5563",
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                          whiteSpace: "nowrap",
                        }}
                      >
                        {m.from}
                      </Typography>
                    </Box>
                    <Typography variant="caption" sx={{ color: "#9CA3AF", flexShrink: 0, fontSize: "0.74rem" }}>
                      {formatDateLabel(m.date)}
                    </Typography>
                  </Box>

                  {/* Bold subject topic */}
                  <Typography
                    variant="subtitle2"
                    sx={{
                      fontWeight: 700,
                      color: "#111827",
                      fontSize: "0.9rem",
                      lineHeight: 1.3,
                      mb: 0.5,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {m.subject || "(No Subject)"}
                  </Typography>

                  <Typography
                    variant="body2"
                    sx={{
                      color: "#6B7280",
                      fontSize: "0.8rem",
                      lineHeight: 1.35,
                      display: "-webkit-box",
                      WebkitLineClamp: 2,
                      WebkitBoxOrient: "vertical",
                      overflow: "hidden",
                    }}
                  >
                    {m.snippet}
                  </Typography>
                </CardContent>
              </Card>
            ))}
          </Box>
        )}
      </Box>

      {/* Enhanced Detailed Email Reader Drawer with Swipe-down-to-close & Bold Topics */}
      <Drawer
        anchor="bottom"
        open={Boolean(selectedEmailId)}
        onClose={() => setSelectedEmailId(null)}
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
          }}
        >
          {/* Swipe down handle bar indicator */}
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
            <IconButton size="small" onClick={() => setSelectedEmailId(null)}>
              <CloseRoundedIcon fontSize="small" />
            </IconButton>
          </Box>
        </Box>

        {/* Reader Content Body */}
        <Box sx={{ flex: 1, overflowY: "auto", p: 3 }}>
          {loadingDetail ? (
            <Box sx={{ display: "flex", justifyContent: "center", py: 10 }}>
              <CircularProgress size={30} sx={{ color: "#DC4C3E" }} />
            </Box>
          ) : emailDetail ? (
            <Box>
              {/* Bold Topic & Subject Title */}
              <Typography
                variant="h6"
                sx={{
                  fontWeight: 800,
                  color: "#111827",
                  fontSize: "1.2rem",
                  lineHeight: 1.35,
                  mb: 2,
                }}
              >
                {emailDetail.subject || "(No Subject)"}
              </Typography>

              {/* Sender & Metadata Card */}
              <Box
                sx={{
                  p: 2,
                  mb: 3,
                  borderRadius: 3,
                  bgcolor: "#F9FAFB",
                  border: "1px solid #F3F4F6",
                  display: "flex",
                  flexDirection: "column",
                  gap: 1,
                }}
              >
                <Box sx={{ display: "flex", alignItems: "center", gap: 1.5 }}>
                  <Box
                    sx={{
                      width: 40,
                      height: 40,
                      borderRadius: "50%",
                      bgcolor: "#1A73E8",
                      color: "#FFFFFF",
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                      fontWeight: 700,
                      fontSize: "1.1rem",
                    }}
                  >
                    {(emailDetail.from || "G").charAt(0).toUpperCase()}
                  </Box>
                  <Box sx={{ minWidth: 0, flex: 1 }}>
                    <Typography variant="subtitle2" sx={{ fontWeight: 700, color: "#111827", fontSize: "0.95rem" }}>
                      From: <span style={{ fontWeight: 600, color: "#374151" }}>{emailDetail.from}</span>
                    </Typography>
                    {emailDetail.to && emailDetail.to.length > 0 && (
                      <Typography variant="caption" sx={{ color: "#6B7280", display: "block", fontSize: "0.78rem" }}>
                        <strong>To:</strong> {emailDetail.to.join(", ")}
                      </Typography>
                    )}
                  </Box>
                </Box>

                <Box sx={{ borderTop: "1px solid #E5E7EB", pt: 1, mt: 0.5, display: "flex", justifyContent: "space-between" }}>
                  <Typography variant="caption" sx={{ color: "#6B7280", fontSize: "0.78rem" }}>
                    <strong>Date:</strong> {emailDetail.date ? new Date(emailDetail.date).toLocaleString() : "Unknown"}
                  </Typography>
                </Box>
              </Box>

              {/* Enhanced Email Content Body */}
              <Box
                sx={{
                  p: 2.5,
                  borderRadius: 3,
                  bgcolor: "#FFFFFF",
                  border: "1px solid #E5E7EB",
                  boxShadow: "0 2px 10px rgba(0,0,0,0.03)",
                }}
              >
                <Typography
                  variant="body1"
                  component="pre"
                  sx={{
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-word",
                    fontFamily: "inherit",
                    fontSize: "0.92rem",
                    lineHeight: 1.65,
                    color: "#1F2937",
                  }}
                >
                  {emailDetail.body_text || "(Empty body content)"}
                </Typography>
              </Box>
            </Box>
          ) : (
            <Typography variant="body2" sx={{ color: "#6B7280", textAlign: "center", py: 8 }}>
              Could not load email body.
            </Typography>
          )}
        </Box>
      </Drawer>
    </Box>
  );

}



