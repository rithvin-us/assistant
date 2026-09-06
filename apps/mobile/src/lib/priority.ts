export type PriorityLevel = "P1" | "P2" | "P3" | "P4";

export const PRIORITY_COLORS: Record<PriorityLevel, string> = {
  P1: "#DC4C3E", // Todoist Red
  P2: "#EB8909", // Orange
  P3: "#246FE0", // Blue
  P4: "#B0B0B0", // Gray
};

export const PRIORITY_BG: Record<PriorityLevel, string> = {
  P1: "rgba(220, 76, 62, 0.12)",
  P2: "rgba(235, 137, 9, 0.10)",
  P3: "rgba(36, 111, 224, 0.10)",
  P4: "rgba(176, 176, 176, 0.08)",
};
