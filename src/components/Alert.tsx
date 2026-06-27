import type { CSSProperties, ReactNode } from "react";
import { Info, CheckCircle, AlertTriangle, AlertCircle, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";

export type AlertTone = "info" | "success" | "warning" | "danger";

const TONE_ICON: Record<AlertTone, LucideIcon> = {
  info: Info,
  success: CheckCircle,
  warning: AlertTriangle,
  danger: AlertCircle,
};

/**
 * Inline alert / notification banner. Restrained product-register styling:
 * a tinted surface, a tone-colored accent stripe, a lucide icon, and an
 * optional title + body. Used wherever an inline message needs to sit in
 * the content flow (errors, confirmations, warnings, notices).
 *
 * The same visual language is shared by the floating Toast stack so the two
 * read as one family. `compact` drops the surface/chrome for toolbar-embedded
 * confirmations (e.g. "Copied to clipboard").
 */
export function Alert({
  tone = "info",
  title,
  children,
  onDismiss,
  compact = false,
  icon,
  className,
  style,
}: {
  tone?: AlertTone;
  title?: ReactNode;
  children?: ReactNode;
  onDismiss?: () => void;
  compact?: boolean;
  icon?: LucideIcon;
  className?: string;
  style?: CSSProperties;
}) {
  const Icon = icon ?? TONE_ICON[tone];
  const cls = ["alert", `alert--${tone}`, compact ? "alert--compact" : "", className ?? ""]
    .filter(Boolean)
    .join(" ");
  return (
    <div
      className={cls}
      role={tone === "danger" ? "alert" : "status"}
      style={style}
    >
      <Icon className="alert__icon" size={compact ? 14 : 16} aria-hidden />
      <div className="alert__body">
        {title != null && title !== "" && <div className="alert__title">{title}</div>}
        {children != null && children !== "" && <div className="alert__text">{children}</div>}
      </div>
      {onDismiss && (
        <button type="button" className="alert__close" onClick={onDismiss} aria-label="Dismiss notification">
          <X size={14} aria-hidden />
        </button>
      )}
    </div>
  );
}
