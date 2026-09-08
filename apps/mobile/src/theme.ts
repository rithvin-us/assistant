/**
 * Material UI theme with zero tap highlight and anti-aliased typography.
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
    MuiCssBaseline: {
      styleOverrides: `
        html, body, #root {
          margin: 0;
          padding: 0;
          width: 100%;
          height: 100%;
          overflow-x: hidden;
        }
        * {
          -webkit-tap-highlight-color: transparent !important;
          -webkit-touch-callout: none !important;
          outline: none !important;
          box-sizing: border-box;
        }
        *:focus, *:focus-visible, *:active {
          outline: none !important;
          box-shadow: none !important;
        }
        body {
          user-select: none;
          -webkit-user-select: none;
          -webkit-font-smoothing: antialiased;
          -moz-osx-font-smoothing: grayscale;
        }
      `,
    },
    MuiPaper: {
      defaultProps: { elevation: 0 },
      styleOverrides: {
        root: { backgroundImage: "none", border: "1px solid rgba(0,0,0,0.08)" },
      },
    },
    MuiButton: {
      defaultProps: { disableElevation: true },
      styleOverrides: { root: { minHeight: 44, WebkitTapHighlightColor: "transparent" } },
    },
    MuiIconButton: {
      styleOverrides: { root: { WebkitTapHighlightColor: "transparent" } },
    },
    MuiListItemButton: { styleOverrides: { root: { minHeight: 48, WebkitTapHighlightColor: "transparent" } } },
  },
});
