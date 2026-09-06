/**
 * Todoist-inspired Four-Tier Priority Checkbox.
 *
 * Implements DESIGN-android.md specification:
 * - Priority colored stroke (P1 red #DC4C3E, P2 orange #EB8909, P3 blue #246FE0, P4 gray #B0B0B0).
 * - Minimum 44dp touch target hit area.
 * - Subtle priority fill tint on hover/press.
 */

import Box from "@mui/material/Box";
import CheckRoundedIcon from "@mui/icons-material/CheckRounded";
import { type PriorityLevel, PRIORITY_COLORS, PRIORITY_BG } from "../lib/priority";

export type { PriorityLevel };


interface TodoistCheckboxProps {
  priority?: PriorityLevel | string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}

export default function TodoistCheckbox({
  priority = "P4",
  checked,
  onChange,
  disabled = false,
}: TodoistCheckboxProps) {
  const pKey = (["P1", "P2", "P3", "P4"].includes(priority) ? priority : "P4") as PriorityLevel;
  const strokeColor = PRIORITY_COLORS[pKey];
  const fillColor = checked ? strokeColor : PRIORITY_BG[pKey];

  return (
    <Box
      onClick={(e) => {
        e.stopPropagation();
        if (!disabled) onChange(!checked);
      }}
      sx={{
        width: 44,
        height: 44,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        cursor: disabled ? "default" : "pointer",
        WebkitTapHighlightColor: "transparent",
        flexShrink: 0,
      }}
    >
      <Box
        sx={{
          width: 20,
          height: 20,
          borderRadius: "50%",
          border: `2px solid ${strokeColor}`,
          bgcolor: fillColor,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          transition: "all 0.18s cubic-bezier(0.16, 1, 0.3, 1)",
          transform: checked ? "scale(1.05)" : "scale(1)",
          "&:hover": {
            transform: disabled ? "none" : "scale(1.1)",
            bgcolor: checked ? strokeColor : PRIORITY_BG[pKey],
          },
        }}
      >
        {checked && (
          <CheckRoundedIcon
            sx={{
              fontSize: 14,
              color: "#FFFFFF",
              strokeWidth: 2,
            }}
          />
        )}
      </Box>
    </Box>
  );
}
