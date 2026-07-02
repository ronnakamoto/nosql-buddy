import { useEffect, useMemo, useRef } from "react";
import {
  Monitor,
  Cpu,
  ExternalLink,
  Keyboard,
  Code2,
  X,
} from "lucide-react";
import logoUrl from "../assets/logo.png";
import type { AppInfo } from "../ipc/commands";

interface AboutScreenProps {
  open: boolean;
  onClose: () => void;
  info: AppInfo | null;
  onOpenShortcuts?: () => void;
}

export function AboutScreen({
  open,
  onClose,
  info,
  onOpenShortcuts,
}: AboutScreenProps) {
  const closeRef = useRef<HTMLButtonElement>(null);
  const year = useMemo(() => new Date().getFullYear(), []);

  const specs = useMemo(
    () => [
      { icon: Monitor, label: "Platform", value: info?.platform ?? "—" },
      { icon: Cpu, label: "Architecture", value: info?.arch ?? "—" },
    ],
    [info],
  );

  useEffect(() => {
    if (!open) return;
    closeRef.current?.focus();
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className="modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="about-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          e.preventDefault();
          onClose();
        }
      }}
    >
      <div className="modal about-dialog" style={{ width: "min(520px, 92vw)" }}>
        <div className="modal__header">
          <div className="modal__heading">
            <h2 className="modal__title" id="about-title">About NoSQLBuddy</h2>
          </div>
          <button
            ref={closeRef}
            className="modal__close"
            onClick={onClose}
            aria-label="Close"
            type="button"
          >
            <X size={16} />
          </button>
        </div>

        <div className="modal__body about-dialog__body">
          <div className="about-dialog__hero">
            <img
              src={logoUrl}
              alt=""
              className="about-dialog__logo"
              draggable={false}
            />
            <h3 className="about-dialog__app-name">NoSQLBuddy</h3>
            <p className="about-dialog__version">
              Version {info?.appVersion ?? "0.1.0"}
            </p>
            <p className="about-dialog__description">
              A modern desktop client for MongoDB. Built for developers who
              need speed, clarity, and control over their document databases.
            </p>
          </div>

          <div className="about-dialog__specs">
            {specs.map((s) => (
              <div key={s.label} className="about-dialog__spec">
                <s.icon size={14} aria-hidden="true" />
                <span className="about-dialog__spec-label">{s.label}</span>
                <span className="about-dialog__spec-value">{s.value}</span>
              </div>
            ))}
          </div>

          <div className="about-dialog__actions">
            {onOpenShortcuts && (
              <button
                className="toolbar-btn"
                onClick={() => {
                  onClose();
                  onOpenShortcuts();
                }}
                title="View keyboard shortcuts"
              >
                <Keyboard size={14} aria-hidden="true" />
                <span>Keyboard shortcuts</span>
              </button>
            )}
            <a
              className="toolbar-btn"
              href="https://github.com"
              target="_blank"
              rel="noopener noreferrer"
              title="View source on GitHub"
            >
              <Code2 size={14} aria-hidden="true" />
              <span>Source code</span>
              <ExternalLink size={12} aria-hidden="true" className="about-dialog__link-icon" />
            </a>
          </div>
        </div>

        <div className="modal__footer about-dialog__footer">
          <span className="about-dialog__footer-line">
            NoSQLBuddy is not affiliated with MongoDB Inc.
          </span>
          <span className="about-dialog__footer-line">
            &copy; {year} NoSQLBuddy. All rights reserved.
          </span>
        </div>
      </div>
    </div>
  );
}
