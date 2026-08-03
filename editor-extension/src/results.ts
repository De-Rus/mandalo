import * as vscode from "vscode";
import type { RequestOutcome, RunResult, SendResult } from "./core/cli";

function nonce(): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let out = "";
  for (let i = 0; i < 32; i += 1) out += alphabet[Math.floor(Math.random() * alphabet.length)];
  return out;
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function statusClass(status: number): string {
  if (status >= 200 && status < 300) return "ok";
  if (status >= 300 && status < 400) return "warn";
  return "bad";
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function prettyBody(body: string): string {
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
}

function testsHtml(outcome: RequestOutcome): string {
  if (outcome.tests.length === 0) return '<p class="empty">This request declares no [[tests]].</p>';
  const items = outcome.tests
    .map((test) => {
      const chip = test.passed ? '<span class="chip ok">pass</span>' : '<span class="chip bad">fail</span>';
      const detail = test.detail ? `<div class="detail">${escapeHtml(test.detail)}</div>` : "";
      return `<li>${chip} ${escapeHtml(test.name)}${detail}</li>`;
    })
    .join("");
  return `<ul class="tests">${items}</ul>`;
}

function headersHtml(outcome: RequestOutcome): string {
  const headers = outcome.response?.headers ?? [];
  if (headers.length === 0) return '<p class="empty">No response headers.</p>';
  const rows = headers
    .map(([key, value]) => `<tr><td class="key">${escapeHtml(key)}</td><td>${escapeHtml(value)}</td></tr>`)
    .join("");
  return `<table>${rows}</table>`;
}

function capturesHtml(outcome: RequestOutcome): string {
  if (outcome.captures.length === 0) return '<p class="empty">No captures.</p>';
  const rows = outcome.captures
    .map(
      (capture) =>
        `<tr><td class="key">${escapeHtml(capture.into)}</td><td>${escapeHtml(capture.value)}</td><td class="key">${escapeHtml(capture.from)}</td><td class="key">${escapeHtml(capture.scope)}</td></tr>`,
    )
    .join("");
  return `<table>${rows}</table>`;
}

export function renderOutcome(outcome: RequestOutcome, subtitle: string): string {
  const response = outcome.response;
  const chips: string[] = [];
  if (outcome.error) {
    chips.push('<span class="chip bad">error</span>');
  } else if (response) {
    chips.push(
      `<span class="chip ${statusClass(response.status)}">${response.status} ${escapeHtml(response.statusText)}</span>`,
    );
    chips.push(`<span class="meta">${response.durationMs} ms</span>`);
    chips.push(`<span class="meta">${formatBytes(response.sizeBytes)}</span>`);
  }
  const failed = outcome.tests.filter((test) => !test.passed).length;
  if (outcome.tests.length > 0) {
    chips.push(
      `<span class="chip ${failed === 0 ? "ok" : "bad"}">${outcome.tests.length - failed}/${outcome.tests.length} tests</span>`,
    );
  }

  const bodyPanel = outcome.error
    ? `<div class="error">${escapeHtml(outcome.error)}</div>`
    : response
      ? response.binary
        ? `<p class="empty">Binary response (${formatBytes(response.sizeBytes)}) — not rendered.</p>`
        : `<pre>${escapeHtml(prettyBody(response.body))}</pre>`
      : '<p class="empty">No response.</p>';

  return `
    <header>
      <span class="title">${escapeHtml(outcome.method.toUpperCase())} ${escapeHtml(outcome.url || outcome.name)}</span>
      ${chips.join("")}
    </header>
    <p class="meta">${escapeHtml(subtitle)}</p>
    <nav>
      <button data-tab="body" aria-selected="true">Body</button>
      <button data-tab="headers">Headers</button>
      <button data-tab="tests">Tests</button>
      <button data-tab="captures">Captures</button>
    </nav>
    <section data-panel="body">${bodyPanel}</section>
    <section data-panel="headers" hidden>${headersHtml(outcome)}</section>
    <section data-panel="tests" hidden>${testsHtml(outcome)}</section>
    <section data-panel="captures" hidden>${capturesHtml(outcome)}</section>
  `;
}

export function renderRun(result: RunResult): string {
  const chipClass = result.failed === 0 ? "ok" : "bad";
  const rows = result.requests
    .map((outcome) => {
      const failed = outcome.tests.filter((test) => !test.passed).length;
      const status = outcome.error
        ? '<span class="chip bad">error</span>'
        : `<span class="chip ${statusClass(outcome.response?.status ?? 0)}">${outcome.response?.status ?? "—"}</span>`;
      const detail = outcome.error
        ? escapeHtml(outcome.error)
        : outcome.tests
            .filter((test) => !test.passed)
            .map((test) => escapeHtml(`${test.name}${test.detail ? ` — ${test.detail}` : ""}`))
            .join("<br>");
      return `<tr><td>${status}</td><td class="key">${escapeHtml(outcome.path)}</td><td>${failed === 0 && !outcome.error ? "" : detail}</td></tr>`;
    })
    .join("");
  return `
    <header>
      <span class="title">${escapeHtml(result.collection)}</span>
      <span class="chip ${chipClass}">${result.passed}/${result.total} passed</span>
      <span class="meta">${result.durationMs} ms</span>
      <span class="meta">${escapeHtml(result.env ?? "no environment")}</span>
    </header>
    <nav><button data-tab="summary" aria-selected="true">Summary</button></nav>
    <section data-panel="summary"><table>${rows}</table></section>
  `;
}

export class ResultsPanel {
  private static current: ResultsPanel | undefined;
  private readonly panel: vscode.WebviewPanel;

  private constructor(private readonly extensionUri: vscode.Uri) {
    this.panel = vscode.window.createWebviewPanel(
      "mandalo.results",
      "Mándalo Response",
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
      {
        enableScripts: true,
        localResourceRoots: [vscode.Uri.joinPath(extensionUri, "media")],
        retainContextWhenHidden: true,
      },
    );
    this.panel.onDidDispose(() => {
      ResultsPanel.current = undefined;
    });
  }

  static show(extensionUri: vscode.Uri, title: string, inner: string): void {
    if (!ResultsPanel.current) ResultsPanel.current = new ResultsPanel(extensionUri);
    ResultsPanel.current.update(title, inner);
  }

  static showSend(extensionUri: vscode.Uri, result: SendResult): void {
    const subtitle = `${result.collection} · ${result.path} · ${result.env ?? "no environment"}`;
    ResultsPanel.show(extensionUri, result.name || result.path, renderOutcome(result, subtitle));
  }

  static showRun(extensionUri: vscode.Uri, result: RunResult): void {
    ResultsPanel.show(extensionUri, `${result.collection} run`, renderRun(result));
  }

  private update(title: string, inner: string): void {
    this.panel.title = `Mándalo · ${title}`;
    this.panel.webview.html = this.html(inner);
    this.panel.reveal(vscode.ViewColumn.Beside, true);
  }

  private html(inner: string): string {
    const webview = this.panel.webview;
    const styleUri = webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "media", "results.css"));
    const scriptUri = webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "media", "results.js"));
    const scriptNonce = nonce();
    const csp = [
      "default-src 'none'",
      `style-src ${webview.cspSource}`,
      `script-src 'nonce-${scriptNonce}'`,
      `font-src ${webview.cspSource}`,
    ].join("; ");
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="${styleUri}">
<title>Mándalo</title>
</head>
<body>
${inner}
<script nonce="${scriptNonce}" src="${scriptUri}"></script>
</body>
</html>`;
  }
}
