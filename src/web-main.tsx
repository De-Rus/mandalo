import "./lib/web/boot";
import React from "react";
import ReactDOM from "react-dom/client";
import { seedIfEmpty } from "./lib/web/mounts";
import { WebShell } from "./lib/web/WebShell";
import "./styles/index.css";
import "./lib/web/web.css";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

async function start(): Promise<void> {
  await seedIfEmpty();
  root.render(
    <React.StrictMode>
      <WebShell />
    </React.StrictMode>,
  );
}

void start().catch((e: unknown) =>
  root.render(
    <div className="web-boot">
      <p>
        Mándalo could not open its local storage:{" "}
        {e instanceof Error ? e.message : String(e)}. Private-browsing windows
        often block IndexedDB — try a normal window, or use the desktop app.
      </p>
    </div>,
  ),
);
