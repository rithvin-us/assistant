/**
 * Connected Accounts Management Screen.
 *
 * Displays connected Google identities, connection status, granted scopes,
 * and enables secure account connection and revocation.
 */

import { useState, useEffect, useCallback } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import IconButton from "@mui/material/IconButton";
import Dialog from "@mui/material/Dialog";
import DialogTitle from "@mui/material/DialogTitle";
import DialogContent from "@mui/material/DialogContent";
import DialogActions from "@mui/material/DialogActions";
import CircularProgress from "@mui/material/CircularProgress";
import Card from "@mui/material/Card";
import CardContent from "@mui/material/CardContent";
import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";
import DeleteOutlineRoundedIcon from "@mui/icons-material/DeleteOutlineRounded";
import GoogleIcon from "@mui/icons-material/Google";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import ErrorOutlineRoundedIcon from "@mui/icons-material/ErrorOutlineRounded";

import type { AccountSummary } from "../api/types";
import {
  fetchGoogleAccounts,
  startGoogleOAuth,
  disconnectGoogleAccount,
} from "../api/google";

interface ConnectionsScreenProps {
  onBack?: () => void;
}

export default function ConnectionsScreen({ onBack }: ConnectionsScreenProps) {
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [connecting, setConnecting] = useState(false);
  const [disconnectTarget, setDisconnectTarget] = useState<AccountSummary | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const loadAccounts = useCallback(async () => {
    try {
      setLoading(true);
      setErrorMessage(null);
      const list = await fetchGoogleAccounts();
      setAccounts(list);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Failed to load accounts";
      setErrorMessage(msg);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    loadAccounts();
  }, [loadAccounts]);

  const handleConnect = async () => {
    try {
      setConnecting(true);
      setErrorMessage(null);
      const res = await startGoogleOAuth();
      if (res.auth_url) {
        window.open(res.auth_url, "_blank");
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Failed to start Google OAuth";
      setErrorMessage(msg);
    } finally {
      setConnecting(false);
    }
  };

  const handleConfirmDisconnect = async () => {
    if (!disconnectTarget) return;
    try {
      await disconnectGoogleAccount(disconnectTarget.id);
      setDisconnectTarget(null);
      await loadAccounts();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Failed to disconnect account";
      setErrorMessage(msg);
    }
  };

  const inferAccountType = (email: string): string => {
    const lower = email.toLowerCase();
    if (lower.includes(".edu") || lower.includes("college") || lower.includes("student")) {
      return "College";
    }
    if (lower.includes("work") || lower.includes("corp") || lower.includes("company")) {
      return "Work";
    }
    return "Personal";
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
              Connected Accounts
            </Typography>
            <Typography variant="caption" sx={{ color: "#808080" }}>
              Google ecosystem & schedules
            </Typography>
          </Box>
        </Box>
        <IconButton onClick={loadAccounts} size="small" sx={{ color: "#606060" }}>
          <RefreshRoundedIcon />
        </IconButton>
      </Box>

      {/* Main Content */}
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

        {/* Connect Action Card */}
        <Card
          variant="outlined"
          sx={{
            mb: 3,
            borderRadius: 3,
            borderColor: "#EBEBEB",
            bgcolor: "#FBFBFB",
          }}
        >
          <CardContent sx={{ p: 2, "&:last-child": { pb: 2 } }}>
            <Typography variant="subtitle2" sx={{ fontWeight: 600, color: "#202020", mb: 0.5 }}>
              Connect Google Account
            </Typography>
            <Typography variant="body2" sx={{ color: "#666666", fontSize: "0.82rem", mb: 2 }}>
              Connect personal, college, or work Google accounts to access Gmail and Google Calendar seamlessly.
            </Typography>
            <Button
              variant="contained"
              disableElevation
              onClick={handleConnect}
              disabled={connecting}
              startIcon={connecting ? <CircularProgress size={16} color="inherit" /> : <GoogleIcon />}
              sx={{
                bgcolor: "#202020",
                color: "#FFFFFF",
                textTransform: "none",
                fontWeight: 600,
                borderRadius: 2,
                px: 2,
                py: 1,
                "&:hover": { bgcolor: "#333333" },
              }}
            >
              {connecting ? "Opening Google Sign-In..." : "Add Google Account"}
            </Button>
          </CardContent>
        </Card>

        {/* Account List */}
        <Typography variant="overline" sx={{ color: "#888888", fontWeight: 700, letterSpacing: 0.8 }}>
          Connected ({accounts.length})
        </Typography>

        {loading ? (
          <Box sx={{ display: "flex", justifyContent: "center", py: 4 }}>
            <CircularProgress size={24} sx={{ color: "#DC4C3E" }} />
          </Box>
        ) : accounts.length === 0 ? (
          <Box sx={{ textAlign: "center", py: 5, color: "#808080" }}>
            <GoogleIcon sx={{ fontSize: 40, color: "#CCCCCC", mb: 1 }} />
            <Typography variant="body2">No Google accounts connected yet.</Typography>
            <Typography variant="caption" sx={{ color: "#AAAAAA" }}>
              Tap Add Google Account above to connect your first account.
            </Typography>
          </Box>
        ) : (
          <Box sx={{ display: "flex", flexDirection: "column", gap: 1.5, mt: 1 }}>
            {accounts.map((acc) => {
              const accountType = inferAccountType(acc.email);
              const isActive = acc.status === "active";
              return (
                <Card
                  key={acc.id}
                  variant="outlined"
                  sx={{
                    borderRadius: 2.5,
                    borderColor: "#EBEBEB",
                    bgcolor: "#FFFFFF",
                    transition: "border-color 0.2s",
                    "&:hover": { borderColor: "#D0D0D0" },
                  }}
                >
                  <CardContent sx={{ p: 2, "&:last-child": { pb: 2 } }}>
                    <Box sx={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between" }}>
                      <Box sx={{ display: "flex", alignItems: "center", gap: 1.25 }}>
                        <Box
                          sx={{
                            width: 36,
                            height: 36,
                            borderRadius: "50%",
                            bgcolor: "#F4F4F4",
                            display: "flex",
                            alignItems: "center",
                            justifyContent: "center",
                            color: "#4285F4",
                          }}
                        >
                          <GoogleIcon fontSize="small" />
                        </Box>
                        <Box>
                          <Typography variant="subtitle2" sx={{ fontWeight: 600, color: "#202020", lineHeight: 1.2 }}>
                            {acc.display_name || acc.email}
                          </Typography>
                          <Typography variant="caption" sx={{ color: "#707070", fontSize: "0.78rem" }}>
                            {acc.email}
                          </Typography>
                        </Box>
                      </Box>
                      <IconButton
                        size="small"
                        onClick={() => setDisconnectTarget(acc)}
                        sx={{ color: "#999999", "&:hover": { color: "#DC4C3E" } }}
                        title="Disconnect account"
                      >
                        <DeleteOutlineRoundedIcon fontSize="small" />
                      </IconButton>
                    </Box>

                    {/* Metadata & Badges */}
                    <Box sx={{ display: "flex", alignItems: "center", gap: 1, mt: 1.5, flexWrap: "wrap" }}>
                      <Chip
                        label={accountType}
                        size="small"
                        sx={{
                          height: 22,
                          fontSize: "0.72rem",
                          fontWeight: 600,
                          bgcolor: "#F0F0F0",
                          color: "#404040",
                        }}
                      />
                      <Chip
                        icon={isActive ? <CheckCircleRoundedIcon sx={{ fontSize: "14px !important" }} /> : <ErrorOutlineRoundedIcon sx={{ fontSize: "14px !important" }} />}
                        label={isActive ? "Active" : acc.status}
                        size="small"
                        color={isActive ? "success" : "default"}
                        variant="outlined"
                        sx={{ height: 22, fontSize: "0.72rem", fontWeight: 500 }}
                      />
                      <Typography variant="caption" sx={{ color: "#AAAAAA", fontSize: "0.72rem" }}>
                        Scopes: Gmail Read, Calendar
                      </Typography>
                    </Box>
                  </CardContent>
                </Card>
              );
            })}
          </Box>
        )}
      </Box>

      {/* Disconnect Confirmation Dialog */}
      <Dialog
        open={Boolean(disconnectTarget)}
        onClose={() => setDisconnectTarget(null)}
        slotProps={{
          paper: { sx: { borderRadius: 3, p: 1 } },
        }}
      >
        <DialogTitle sx={{ fontWeight: 700, color: "#202020", pb: 1 }}>
          Disconnect Account?
        </DialogTitle>
        <DialogContent>
          <Typography variant="body2" sx={{ color: "#555555" }}>
            Are you sure you want to disconnect <strong>{disconnectTarget?.email}</strong>?
            Local access credentials will be revoked immediately. Your Google data will not be deleted from Google.
          </Typography>
        </DialogContent>
        <DialogActions sx={{ pt: 1, px: 2, pb: 1.5 }}>
          <Button onClick={() => setDisconnectTarget(null)} sx={{ color: "#666666", textTransform: "none", fontWeight: 600 }}>
            Cancel
          </Button>
          <Button
            onClick={handleConfirmDisconnect}
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
            Disconnect
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}
