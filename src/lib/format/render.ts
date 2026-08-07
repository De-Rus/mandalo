import type { Auth, FormDataRowModel, RequestModel } from "./model";

const GRAPHQL_MARKER = "X-REQUEST-TYPE";
const PROTO_KEY = "proto";

export class RenderError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RenderError";
  }
}

export function newlineOf(source: string): string {
  return source.includes("\r\n") ? "\r\n" : "\n";
}

/**
 * Everything a request can carry that the text formats have no line for. Saving one
 * has to fail rather than drop it, or an edit in a GUI would quietly delete what the
 * file never showed. The Rust twin is `text_format::reject_inexpressible`.
 */
export function rejectInexpressible(request: RequestModel, extension: string): void {
  if (
    (request.tests !== undefined && request.tests.length > 0) ||
    (request.captures !== undefined && request.captures.length > 0)
  ) {
    throw new RenderError(
      `a .${extension} file cannot carry declarative tests or captures — assert and capture from a \`> {% … %}\` response script instead`,
    );
  }
}

function descriptionComment(description: string | undefined, nl: string): string {
  const text = (description ?? "").trim();
  if (text === "") return "";
  return text
    .split("\n")
    .map((line) => `# ${line.replace(/\s+$/, "")}${nl}`)
    .join("");
}

function authHeader(auth: Auth): [string, string] | null {
  switch (auth.type) {
    case "none":
      return null;
    case "inherited":
      return authHeader(auth.auth);
    case "bearer":
      return ["Authorization", `Bearer ${auth.token}`];
    case "basic":
      if (auth.username.includes(":")) {
        throw new RenderError(
          `a basic-auth username cannot contain a colon in a .http file: ${JSON.stringify(auth.username)}`,
        );
      }
      return ["Authorization", `Basic ${auth.username}:${auth.password}`];
    case "apikey":
      if (auth.placement === "header") return [auth.key, auth.value];
      throw new RenderError(
        `a .http file writes an api key as a header or a query parameter, not as ${JSON.stringify(auth.placement)} — put it in the URL instead`,
      );
  }
}

/** One field per line. The Rust twin is `render_form_fields`. */
function renderFormdata(rows: readonly FormDataRowModel[]): string {
  let out = "";
  for (const row of rows) {
    if (row.key.trim() !== row.key || row.key === "") {
      throw new RenderError(
        `the form field name ${JSON.stringify(row.key)} cannot be empty or padded with spaces in a .http file`,
      );
    }
    if (/[=<\n\r]/.test(row.key)) {
      throw new RenderError(
        `the form field name ${JSON.stringify(row.key)} cannot carry \`=\`, \`<\` or a line break in a .http file`,
      );
    }
    if (row.files !== undefined && row.files.length > 0) {
      for (const path of row.files) {
        if (path.includes(";")) {
          throw new RenderError(
            `the form file path ${JSON.stringify(path)} cannot carry a semicolon in a .http file — \`;\` starts the \`type=\` parameter`,
          );
        }
        if (path.includes("<")) {
          throw new RenderError(
            `the form file path ${JSON.stringify(path)} cannot carry \`<\` in a .http file — \`<\` starts the next file on the field`,
          );
        }
      }
      out += `${row.key} =`;
      for (const path of row.files) out += ` < ${path}`;
      if (row.contentType !== undefined) out += `; type=${row.contentType}`;
      out += "\n";
      continue;
    }
    const value = row.value ?? "";
    if (/^<([ \t.]|$)/.test(value)) {
      throw new RenderError(
        `the form field ${JSON.stringify(row.key)} has a value starting with \`<\`, which a .http file would read back as a file reference`,
      );
    }
    if (/[\n\r]/.test(value)) {
      throw new RenderError(
        `the form field ${JSON.stringify(row.key)} has a value spanning more than one line, which a .http file cannot write — keep it on one line`,
      );
    }
    if (value.trim() !== value) {
      throw new RenderError(
        `the form field ${JSON.stringify(row.key)} has a value padded with spaces, which a .http file cannot write`,
      );
    }
    out += `${row.key} = ${value}\n`;
  }
  return out.replace(/\n+$/, "");
}

function renderBody(request: RequestModel): string | null {
  if (request.bodyFile !== undefined) return `< ${request.bodyFile}`;
  if (request.graphql) {
    const query = request.graphql.query.replace(/\s+$/, "");
    const variables = request.graphql.variables.trim();
    return variables === "" ? query : `${query}\n\n${variables}`;
  }
  if (request.formdata !== undefined) {
    return request.formdata.length === 0 ? null : renderFormdata(request.formdata);
  }
  const text = request.body ?? "";
  return text.trim() === "" ? null : text;
}

function script(text: string | undefined): string | null {
  const trimmed = (text ?? "").trim();
  return trimmed === "" ? null : trimmed;
}

function bodyLines(body: string, nl: string): string {
  return body
    .split("\n")
    .map((line) => `${line.replace(/\r$/, "")}${nl}`)
    .join("");
}

function renderHttp(request: RequestModel, nl: string): string {
  rejectInexpressible(request, "http");
  let out = `### ${request.name}${nl}`;
  out += descriptionComment(request.description, nl);
  const pre = script(request.scripts?.pre);
  if (pre !== null) out += `< {%${nl}${pre}${nl}%}${nl}`;
  if ((request.auth ?? { type: "none" }).type === "inherited") out += `# @auth inherited${nl}`;
  if (request.method !== "GET") out += `${request.method} `;
  out += `${request.url}${nl}`;

  if (request.kind === "graphql") out += `${GRAPHQL_MARKER}: GraphQL${nl}`;
  const auth = authHeader(request.auth ?? { type: "none" });
  if (auth !== null) out += `${auth[0]}: ${auth[1]}${nl}`;
  for (const [name, value] of request.headers ?? []) out += `${name}: ${value}${nl}`;
  if (
    request.formdata !== undefined &&
    request.formdata.length > 0 &&
    !(request.headers ?? []).some(([k]) => k.toLowerCase() === "content-type")
  ) {
    out += `Content-Type: multipart/form-data${nl}`;
  }

  const body = renderBody(request);
  if (body !== null) out += nl + bodyLines(body, nl);
  const post = script(request.scripts?.post);
  if (post !== null) out += `${nl}> {%${nl}${post}${nl}%}${nl}`;
  return out;
}

function renderGrpc(request: RequestModel, nl: string): string {
  const grpc = request.grpc;
  if (!grpc) throw new RenderError("this gRPC request carries no gRPC call");
  rejectInexpressible(request, "grpc");
  let out = `### ${request.name}${nl}`;
  out += descriptionComment(request.description, nl);
  const pre = script(request.scripts?.pre);
  if (pre !== null) out += `< {%${nl}${pre}${nl}%}${nl}`;
  out += `${request.url}/${grpc.service}/${grpc.method}${nl}`;
  for (const path of grpc.protoPaths) out += `${PROTO_KEY}: ${path}${nl}`;
  for (const [key, value] of grpc.metadata) out += `${key}: ${value}${nl}`;
  const message = grpc.message.trim();
  if (message !== "") out += nl + bodyLines(message, nl);
  const post = script(request.scripts?.post);
  if (post !== null) out += `${nl}> {%${nl}${post}${nl}%}${nl}`;
  return out;
}

/** One request as the text its format writes. The Rust twin is `render_request`. */
export function renderRequest(request: RequestModel, nl = "\n"): string {
  if (request.kind === "grpc") return renderGrpc(request, nl);
  if (request.kind === "http" || request.kind === "graphql") return renderHttp(request, nl);
  throw new RenderError(`unsupported request kind: ${request.kind}`);
}

export function renderFile(requests: readonly RequestModel[], nl = "\n"): string {
  return requests.map((request) => renderRequest(request, nl)).join(nl);
}

export function extensionForKind(kind: string): "http" | "grpc" {
  if (kind === "http" || kind === "graphql") return "http";
  if (kind === "grpc") return "grpc";
  throw new RenderError(`unsupported request kind: ${kind}`);
}
