import type { CSSProperties, ReactNode } from "react";

// ---------- 设计 token ----------
export const theme = {
  colors: {
    bg: "#1e1e1e",
    panel: "#252525",
    elevated: "#2a2a2a",
    hover: "#333333",
    border: "#3a3a3a",
    borderHover: "#4a4a4a",
    text: "#eee",
    textDim: "#aaa",
    textMuted: "#666",
    textDisabled: "#555",
    accent: "#4a9eff",
  },
  radius: { sm: 4, md: 6, lg: 8 },
  controlHeight: 32,
  fontSize: 13,
};

// ---------- 图标 ----------
export type IconName =
  | "prev"
  | "next"
  | "play"
  | "pause"
  | "sequence"
  | "loop"
  | "single"
  | "volume"
  | "volumeLow"
  | "volumeMute";

const ICON_PATHS: Record<IconName, string> = {
  prev: "M6 6h2v12H6V6zm3.5 6L18 6v12l-8.5-6z",
  next: "M16 6h2v12h-2V6zM6 6v12l8.5-6L6 6z",
  play: "M8 5v14l11-7L8 5z",
  pause: "M6 5h4v14H6V5zm8 0h4v14h-4V5z",
  sequence: "M12 4l-1.41 1.41L16.17 11H4v2h12.17l-5.58 5.59L12 20l8-8z",
  loop: "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z",
  single:
    "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4zm-4-2V9h-1l-2 1v1h1.5v4H13z",
  volume:
    "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z",
  volumeLow:
    "M5 9v6h4l5 5V4L9 9H5zm11.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02z",
  volumeMute:
    "M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4L9.91 6.09 12 8.18V4z",
};

export function Icon({ name, size = 18 }: { name: IconName; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      style={{ display: "block", pointerEvents: "none" }}
    >
      <path d={ICON_PATHS[name]} />
    </svg>
  );
}

// ---------- 通用按钮 ----------
export function Button({
  onClick,
  disabled,
  children,
  variant = "default",
  style,
}: {
  onClick?: () => void;
  disabled?: boolean;
  children: ReactNode;
  variant?: "default" | "primary";
  style?: CSSProperties;
}) {
  const isPrimary = variant === "primary";
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={isPrimary ? "ui-btn ui-btn-primary" : "ui-btn"}
      style={style}
    >
      {children}
    </button>
  );
}

// ---------- 图标按钮 ----------
export function IconButton({
  onClick,
  disabled,
  title,
  active,
  children,
}: {
  onClick?: () => void;
  disabled?: boolean;
  title?: string;
  active?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={title}
      className="ui-icon-btn"
      style={{ color: active ? theme.colors.accent : undefined }}
    >
      {children}
    </button>
  );
}

// ---------- 输入框 ----------
export function TextInput({
  value,
  onChange,
  placeholder,
  style,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  style?: CSSProperties;
}) {
  return (
    <input
      type="text"
      className="ui-input"
      value={value}
      placeholder={placeholder}
      onChange={(e) => onChange(e.target.value)}
      style={style}
    />
  );
}

// ---------- 下拉框 ----------
export function Select({
  value,
  onChange,
  options,
  style,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
  style?: CSSProperties;
}) {
  return (
    <select
      className="ui-input ui-select"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      style={style}
    >
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

// ---------- 滑块 ----------
export function RangeSlider({
  value,
  max,
  min = 0,
  step = 0.01,
  onChange,
  onMouseDown,
  onMouseUp,
  disabled,
  style,
}: {
  value: number;
  max: number;
  min?: number;
  step?: number;
  onChange: (v: number) => void;
  onMouseDown?: () => void;
  onMouseUp?: (v: number) => void;
  disabled?: boolean;
  style?: CSSProperties;
}) {
  const pct = max > min ? ((value - min) / (max - min)) * 100 : 0;
  const fill = disabled ? theme.colors.textDisabled : theme.colors.accent;

  return (
    <input
      type="range"
      className="range-slider"
      min={min}
      max={max}
      step={step}
      value={value}
      onChange={(e) => onChange(Number(e.target.value))}
      onMouseDown={onMouseDown}
      onMouseUp={(e) =>
        onMouseUp?.(Number((e.target as HTMLInputElement).value))
      }
      disabled={disabled}
      style={{
        background: `linear-gradient(to right, ${fill} 0%, ${fill} ${pct}%, ${theme.colors.border} ${pct}%, ${theme.colors.border} 100%)`,
        ...style,
      }}
    />
  );
}