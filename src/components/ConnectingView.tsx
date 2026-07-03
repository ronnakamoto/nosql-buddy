import { Server, Check } from "lucide-react";

/**
 * Canonical connection-establishment phases, in order. The backend emits
 * `connection-progress` events for the first four; the fifth (`collections`)
 * is driven by the frontend's `listCollections` loop. Keeping the list here
 * (not in the backend) means the stepper always renders all steps immediately,
 * so the user sees the full shape of the work up front instead of steps
 * popping in as events arrive.
 */
const PHASES: { phase: string; label: string }[] = [
  { phase: "resolve", label: "Resolving connection string" },
  { phase: "authenticate", label: "Authenticating with the server" },
  { phase: "metadata", label: "Reading deployment metadata" },
  { phase: "discover", label: "Discovering databases and statistics" },
  { phase: "collections", label: "Loading collections" },
];

export interface ConnectingViewProps {
  /** Display name of the profile being connected to. */
  profileName: string;
  /** Masked URI hint shown under the name, when available. */
  maskedUri?: string | null;
  /** Map of phase id → status, updated live as progress events arrive. */
  phaseStatus: Record<string, "active" | "done">;
}

/**
 * Shown in the main workspace while a connection is being established.
 * Replaces the blank/empty state so the user can see the connect sequence
 * progressing instead of staring at a frozen screen for the 1–3s (sometimes
 * longer on Atlas) that the TLS + SCRAM handshake + discovery takes.
 */
export function ConnectingView({
  profileName,
  maskedUri,
  phaseStatus,
}: ConnectingViewProps) {
  // The active step is the last phase marked "active", or — if a later phase
  // has already started — every earlier phase is treated as done. This makes
  // the stepper robust to a missing "done" tick on fast local servers.
  const activeIndex = PHASES.reduce(
    (acc, p, i) => (phaseStatus[p.phase] ? i : acc),
    -1,
  );

  return (
    <div className="connecting" role="status" aria-live="polite">
      <div className="connecting__card">
        <header className="connecting__header">
          <div className="connecting__server-id">
            <Server size={18} className="connecting__server-icon" aria-hidden="true" />
            <div className="connecting__server-text">
              <h1 className="connecting__title">Connecting to {profileName}</h1>
              {maskedUri ? (
                <span className="connecting__meta">{maskedUri}</span>
              ) : (
                <span className="connecting__meta">Establishing connection…</span>
              )}
            </div>
          </div>
        </header>

        <ol className="connecting__steps" aria-label="Connection progress">
          {PHASES.map((p, i) => {
            const status = phaseStatus[p.phase];
            const isDone = status === "done" || (activeIndex > i);
            const isActive = status === "active" && activeIndex === i;
            return (
              <li
                key={p.phase}
                className={
                  "connecting__step" +
                  (isDone ? " connecting__step--done" : "") +
                  (isActive ? " connecting__step--active" : "")
                }
                aria-current={isActive ? "step" : undefined}
              >
                <span className="connecting__step-marker" aria-hidden="true">
                  {isDone ? (
                    <Check size={13} className="connecting__check" />
                  ) : isActive ? (
                    <span className="connecting__spinner" />
                  ) : (
                    <span className="connecting__dot" />
                  )}
                </span>
                <span className="connecting__step-label">{p.label}</span>
              </li>
            );
          })}
        </ol>
      </div>
    </div>
  );
}
