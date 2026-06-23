import { Component, ErrorInfo, ReactNode } from "react";

/**
 * Error boundary that catches render errors and shows a recovery message
 * instead of crashing to a white screen.
 *
 * Wrap any component that might throw during render. When an error is caught,
 * the boundary renders a fallback with a "Reload" button that resets state.
 */
interface Props {
  children: ReactNode;
  label?: string;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export default class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`ErrorBoundary [${this.props.label ?? "unknown"}]:`, error, info);
  }

  handleReload = () => {
    this.setState({ hasError: false, error: null });
  };

  render() {
    if (this.state.hasError) {
      return (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            height: "100%",
            padding: "24px",
            gap: "12px",
            fontFamily: "monospace",
            fontSize: "13px",
            textAlign: "center",
          }}
        >
          <p style={{ color: "var(--text, #c33)", fontWeight: "bold" }}>
            {this.props.label ?? "Component"} crashed
          </p>
          <p style={{ opacity: 0.7, maxWidth: "400px", wordBreak: "break-word" }}>
            {this.state.error?.message ?? "Unknown error"}
          </p>
          <button
            onClick={this.handleReload}
            style={{
              padding: "6px 16px",
              fontSize: "12px",
              cursor: "pointer",
              border: "1px solid var(--border, #333)",
              borderRadius: "4px",
              background: "var(--bg-secondary, #1a1a2e)",
            }}
          >
            Try again
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
