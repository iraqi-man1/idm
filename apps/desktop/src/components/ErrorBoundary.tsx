import { Component, type ReactNode } from "react";

interface State {
  error: Error | null;
}

/** Last-resort error screen so a rendering bug never leaves a blank window. */
export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: { componentStack?: string | null }) {
    console.error("UI error", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
        <p className="text-base font-semibold">Something went wrong in the interface.</p>
        <p className="max-w-lg font-mono text-xs text-muted-foreground" data-selectable>
          {this.state.error.message}
        </p>
        <button
          className="rounded-md bg-primary px-3 py-1.5 text-[13px] font-medium text-primary-foreground"
          onClick={() => window.location.reload()}
        >
          Reload
        </button>
      </div>
    );
  }
}
