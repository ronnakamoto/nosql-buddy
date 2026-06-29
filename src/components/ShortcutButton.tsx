import { forwardRef } from "react";
import { formatShortcut, parseShortcut, type ShortcutKeys } from "../lib/shortcutUtils";

interface ShortcutButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  children: React.ReactNode;
  shortcut?: string | ShortcutKeys[];
  variant?: "primary" | "ghost" | "danger" | "default";
  size?: "sm" | "md" | "lg";
}

export const ShortcutButton = forwardRef<HTMLButtonElement, ShortcutButtonProps>(
  ({ children, shortcut, variant = "default", size = "md", className, ...props }, ref) => {
    const baseClasses = "shortcut-btn";
    const variantClasses = {
      primary: "shortcut-btn--primary",
      ghost: "shortcut-btn--ghost", 
      danger: "shortcut-btn--danger",
      default: "shortcut-btn--default"
    };
    const sizeClasses = {
      sm: "shortcut-btn--sm",
      md: "shortcut-btn--md", 
      lg: "shortcut-btn--lg"
    };

    // Format the shortcut for display
    const displayShortcut = shortcut 
      ? typeof shortcut === 'string' 
        ? formatShortcut(parseShortcut(shortcut))
        : formatShortcut(shortcut)
      : undefined;

    return (
      <button
        ref={ref}
        className={`${baseClasses} ${variantClasses[variant]} ${sizeClasses[size]} ${className || ""}`}
        {...props}
      >
        <span className="shortcut-btn__content">{children}</span>
        {displayShortcut && (
          <kbd className="shortcut-btn__kbd">{displayShortcut}</kbd>
        )}
      </button>
    );
  }
);

ShortcutButton.displayName = "ShortcutButton";