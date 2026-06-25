import type { CSSProperties, ReactNode } from "react";

/**
 * Shared UI primitives for the redesigned audit system.
 *
 * One cohesive visual language built on the app's theme tokens
 * (theme.css). Used by the mode chooser, dev flow, production flow,
 * and settings so every audit surface looks and behaves the same.
 */

// ─── Card ───────────────────────────────────────────────────────────────

export function Card({
  children,
  style,
  padded = true,
}: {
  children: ReactNode;
  style?: CSSProperties;
  padded?: boolean;
}) {
  return (
    <div
      style={{
        background: "var(--surface)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-lg)",
        padding: padded ? "var(--space-4)" : undefined,
        ...style,
      }}
    >
      {children}
    </div>
  );
}

export function CardHeader({
  title,
  subtitle,
  icon,
  actions,
}: {
  title: string;
  subtitle?: string;
  icon?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-2)",
        marginBottom: "var(--space-3)",
      }}
    >
      {icon && <span style={{ fontSize: "var(--font-size-lg)" }}>{icon}</span>}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontSize: "var(--font-size-sm)",
            fontWeight: 600,
            color: "var(--ink)",
          }}
        >
          {title}
        </div>
        {subtitle && (
          <div
            style={{
              fontSize: "var(--font-size-xs)",
              color: "var(--ink-faint)",
              marginTop: "2px",
            }}
          >
            {subtitle}
          </div>
        )}
      </div>
      {actions}
    </div>
  );
}

// ─── Badge ──────────────────────────────────────────────────────────────

type BadgeTone = "neutral" | "accent" | "success" | "warning" | "danger" | "info";

const badgeTone: Record<BadgeTone, CSSProperties> = {
  neutral: { background: "var(--surface-3)", color: "var(--ink-muted)" },
  accent: { background: "var(--accent-100)", color: "var(--accent-700)" },
  success: { background: "color-mix(in oklch, var(--success-500) 18%, transparent)", color: "var(--success-500)" },
  warning: { background: "color-mix(in oklch, var(--warning-500) 20%, transparent)", color: "var(--warning-500)" },
  danger: { background: "color-mix(in oklch, var(--danger-500) 18%, transparent)", color: "var(--danger-500)" },
  info: { background: "color-mix(in oklch, var(--info-500) 18%, transparent)", color: "var(--info-500)" },
};

export function Badge({
  children,
  tone = "neutral",
  dot = false,
}: {
  children: ReactNode;
  tone?: BadgeTone;
  dot?: boolean;
}) {
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "6px",
        padding: "3px 10px",
        borderRadius: "999px",
        fontSize: "var(--font-size-xs)",
        fontWeight: 600,
        letterSpacing: "0.01em",
        whiteSpace: "nowrap",
        ...badgeTone[tone],
      }}
    >
      {dot && (
        <span
          style={{
            width: "6px",
            height: "6px",
            borderRadius: "50%",
            background: "currentColor",
          }}
        />
      )}
      {children}
    </span>
  );
}

// ─── Button ─────────────────────────────────────────────────────────────

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

const buttonVariant: Record<ButtonVariant, CSSProperties> = {
  primary: {
    background: "var(--accent-600)",
    color: "white",
    border: "1px solid var(--accent-700)",
  },
  secondary: {
    background: "var(--surface-2)",
    color: "var(--ink)",
    border: "1px solid var(--border)",
  },
  ghost: {
    background: "transparent",
    color: "var(--ink-muted)",
    border: "1px solid transparent",
  },
  danger: {
    background: "color-mix(in oklch, var(--danger-500) 12%, transparent)",
    color: "var(--danger-500)",
    border: "1px solid color-mix(in oklch, var(--danger-500) 30%, transparent)",
  },
};

export function Button({
  children,
  variant = "secondary",
  loading = false,
  disabled = false,
  onClick,
  style,
  title,
}: {
  children: ReactNode;
  variant?: ButtonVariant;
  loading?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  style?: CSSProperties;
  title?: string;
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      disabled={disabled || loading}
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        gap: "var(--space-2)",
        padding: "7px 14px",
        borderRadius: "var(--radius-md)",
        fontSize: "var(--font-size-sm)",
        fontWeight: 500,
        cursor: disabled || loading ? "not-allowed" : "pointer",
        opacity: disabled || loading ? 0.6 : 1,
        transition: "background 0.12s ease, border-color 0.12s ease, transform 0.05s ease",
        ...buttonVariant[variant],
        ...style,
      }}
    >
      {loading && <Spinner size={13} />}
      {children}
    </button>
  );
}

// ─── Spinner ────────────────────────────────────────────────────────────

export function Spinner({ size = 16 }: { size?: number }) {
  return (
    <span
      style={{
        display: "inline-block",
        width: size,
        height: size,
        borderRadius: "50%",
        border: "2px solid var(--border-strong)",
        borderTopColor: "var(--accent-500)",
        animation: "audit-spin 0.7s linear infinite",
      }}
    />
  );
}

// ─── Stat ───────────────────────────────────────────────────────────────

export function Stat({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "2px", minWidth: 0 }}>
      <span
        style={{
          fontSize: "var(--font-size-xs)",
          color: "var(--ink-faint)",
          textTransform: "uppercase",
          letterSpacing: "0.04em",
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontSize: "var(--font-size-md)",
          fontWeight: 600,
          color: "var(--ink)",
          fontFamily: mono ? "var(--font-mono)" : "var(--font-sans)",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {value}
      </span>
    </div>
  );
}

// ─── ProgressBar ────────────────────────────────────────────────────────

export function ProgressBar({
  current,
  max,
  tone = "accent",
}: {
  current: number;
  max: number;
  tone?: "accent" | "success";
}) {
  const pct = max > 0 ? Math.min(100, (current / max) * 100) : 0;
  const color = tone === "success" ? "var(--success-500)" : "var(--accent-500)";
  return (
    <div
      style={{
        height: "6px",
        background: "var(--surface-3)",
        borderRadius: "999px",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          height: "100%",
          width: `${pct}%`,
          background: color,
          borderRadius: "999px",
          transition: "width 0.3s ease",
        }}
      />
    </div>
  );
}

// ─── EmptyState ─────────────────────────────────────────────────────────

export function EmptyState({
  icon,
  title,
  body,
  action,
}: {
  icon?: ReactNode;
  title: string;
  body?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        textAlign: "center",
        gap: "var(--space-3)",
        padding: "var(--space-8) var(--space-4)",
      }}
    >
      {icon && (
        <div style={{ fontSize: "2rem", opacity: 0.5, lineHeight: 1 }}>{icon}</div>
      )}
      <div style={{ fontSize: "var(--font-size-md)", fontWeight: 600, color: "var(--ink)" }}>
        {title}
      </div>
      {body && (
        <div
          style={{
            fontSize: "var(--font-size-sm)",
            color: "var(--ink-muted)",
            maxWidth: "420px",
            lineHeight: "var(--line-height-normal)",
          }}
        >
          {body}
        </div>
      )}
      {action}
    </div>
  );
}

// ─── KeyValue ───────────────────────────────────────────────────────────

export function KeyValue({
  label,
  value,
  mono = true,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
}) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "2px",
        padding: "var(--space-2) 0",
        borderBottom: "1px solid var(--border)",
      }}
    >
      <span
        style={{
          fontSize: "var(--font-size-xs)",
          color: "var(--ink-faint)",
          textTransform: "uppercase",
          letterSpacing: "0.04em",
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontSize: "var(--font-size-sm)",
          fontFamily: mono ? "var(--font-mono)" : "var(--font-sans)",
          color: "var(--ink)",
          wordBreak: "break-all",
        }}
      >
        {value}
      </span>
    </div>
  );
}

// ─── Alert ──────────────────────────────────────────────────────────────

export function Alert({
  tone = "info",
  children,
}: {
  tone?: "info" | "success" | "warning" | "danger";
  children: ReactNode;
}) {
  const toneStyle: Record<string, CSSProperties> = {
    info: { background: "color-mix(in oklch, var(--info-500) 10%, transparent)", color: "var(--info-500)" },
    success: { background: "color-mix(in oklch, var(--success-500) 10%, transparent)", color: "var(--success-500)" },
    warning: { background: "color-mix(in oklch, var(--warning-500) 12%, transparent)", color: "var(--warning-500)" },
    danger: { background: "color-mix(in oklch, var(--danger-500) 10%, transparent)", color: "var(--danger-500)" },
  };
  return (
    <div
      style={{
        display: "flex",
        gap: "var(--space-2)",
        padding: "var(--space-2) var(--space-3)",
        borderRadius: "var(--radius-md)",
        fontSize: "var(--font-size-sm)",
        lineHeight: "var(--line-height-normal)",
        ...toneStyle[tone],
      }}
    >
      {children}
    </div>
  );
}

// ─── Spinner keyframes (injected once) ──────────────────────────────────

let spinInjected = false;
export function injectAuditKeyframes() {
  if (spinInjected) return;
  spinInjected = true;
  const style = document.createElement("style");
  style.textContent = `
    @keyframes audit-spin { to { transform: rotate(360deg); } }
    @keyframes audit-fade-in { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: none; } }
    @keyframes audit-pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.4; } }
  `;
  document.head.appendChild(style);
}
