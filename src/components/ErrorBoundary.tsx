import { Component, ErrorInfo, ReactNode } from "react";
import { Alert } from "./Alert";

/**
 * Error boundary that catches render errors and shows a recovery message
 * instead of crashing to a white screen.
 *
 * Wrap any component that might throw during render. When an error is caught,
 * the boundary renders an on-brand fallback (a danger Alert with the message
 * and a "Try again" button that resets state) using the shared design system.
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
            justifyContent: "center",
            gap: "var(--space-4)",
            height: "100%",
            padding: "var(--space-6)",
            maxWidth: 480,
            margin: "0 auto",
          }}
        >
          <Alert tone="danger" title={`${this.props.label ?? "Component"} crashed`}>
            {this.state.error?.message ?? "Unknown error"}
          </Alert>
          <div style={{ display: "flex", justifyContent: "flex-end" }}>
            <button className="btn btn--primary" onClick={this.handleReload}>
              Try again
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
