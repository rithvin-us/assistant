/**
 * Material UI theme.
 *
 * Light mode is strictly enforced across the application.
 */

import { createTheme } from "@mui/material/styles";

export const theme = createTheme({
  palette: {
    mode: "light",
    background: { default: "#f8fafc", paper: "#ffffff" },
    primary: { main: "#2563eb" },
    secondary: { main: "#475569" },
    warning: { main: "#d97706" },
    error: { main: "#dc2626" },
    success: { main: "#16a34a" },
    text: { primary: "#0f172a", secondary: "#64748b" },
    divider: "rgba(0, 0, 0, 0.08)",
  },
  shape: { borderRadius: 12 },
  typography: {
    fontFamily: [
      "Inter",
      "system-ui",
      "-apple-system",
      "Segoe UI",
      "Roboto",
      "sans-serif",
    ].join(","),
    h1: { fontSize: "1.5rem", fontWeight: 600, letterSpacing: "-0.01em" },
    h2: { fontSize: "1.125rem", fontWeight: 600 },
    body2: { lineHeight: 1.5 },
    button: { textTransform: "none", fontWeight: 600 },
  },
  components: {
    MuiPaper: {
      defaultProps: { elevation: 0 },
      styleOverrides: {
        root: { backgroundImage: "none", border: "1px solid rgba(0,0,0,0.08)" },
      },
    },
    MuiButton: {
      defaultProps: { disableElevation: true },
      styleOverrides: { root: { minHeight: 44 } },
    },
    MuiListItemButton: { styleOverrides: { root: { minHeight: 48 } } },
  },
});
