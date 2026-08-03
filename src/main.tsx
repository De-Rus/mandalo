import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ACTIVE_KEY } from "./lib/collection";
import "./styles/index.css";

interface BoundaryState {
  error: Error | null;
}

class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  BoundaryState
> {
  state: BoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): BoundaryState {
    return { error };
  }

  resetView = () => {
    localStorage.removeItem(ACTIVE_KEY);
    localStorage.removeItem("mandalo.tabs.v1");
    location.reload();
  };

  render() {
    if (this.state.error) {
      return (
        <div className="crash-panel">
          <h1>Something went wrong</h1>
          <p className="crash-message">{this.state.error.message}</p>
          <button className="btn btn-primary" onClick={this.resetView}>
            Reset view
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
