import { describe, expect, it } from "vitest";
import { requestLenses } from "../../src/core/lenses";
import {
  isTextRequestPath,
  requestFilePath,
  requestPathAt,
  scanTextRequests,
  textFileKind,
} from "../../../src/lib/format/textFormat";

const MULTI = `@host = api.dev

### Login
POST https://{{host}}/auth/login
Content-Type: application/json

{ "user": "ada" }

> {%
pm.environment.set("token", pm.response.json().token);
%}

### Get profile
GET https://{{host}}/me
Authorization: Bearer {{token}}
`;

describe("file kinds", () => {
  it("claims .http, .rest and .grpc and nothing else", () => {
    expect(textFileKind("a/b/api.http")).toBe("http");
    expect(textFileKind("API.REST")).toBe("http");
    expect(textFileKind("mock.grpc")).toBe("grpc");
    expect(textFileKind("collection.toml")).toBeUndefined();
    expect(textFileKind("noextension")).toBeUndefined();
    expect(isTextRequestPath("auth/login.http")).toBe(true);
    expect(isTextRequestPath("auth/login.toml")).toBe(false);
  });

  it("splits the block index off a request path", () => {
    expect(requestFilePath("auth/login.http#2")).toBe("auth/login.http");
    expect(requestFilePath("ping.toml")).toBe("ping.toml");
    expect(requestFilePath("weird#dir/ping.toml")).toBe("weird#dir/ping.toml");
    expect(requestPathAt("auth/login.http", 3)).toBe("auth/login.http#3");
  });
});

describe("scanTextRequests", () => {
  it("returns one entry per block, indexed in file order", () => {
    expect(scanTextRequests(MULTI, "http")).toEqual([
      { index: 0, name: "Login", method: "POST", url: "https://{{host}}/auth/login", kind: "http", lineNumber: 2 },
      { index: 1, name: "Get profile", method: "GET", url: "https://{{host}}/me", kind: "http", lineNumber: 12 },
    ]);
  });

  it("reads a CRLF file exactly like an LF one", () => {
    expect(scanTextRequests(MULTI.replace(/\n/g, "\r\n"), "http")).toEqual(
      scanTextRequests(MULTI, "http"),
    );
  });

  it("keeps the last block when the file has no trailing newline", () => {
    const blocks = scanTextRequests("### One\nGET https://a.dev/1\n\n### Two\nGET https://a.dev/2", "http");
    expect(blocks.map((block) => block.name)).toEqual(["One", "Two"]);
    expect(blocks[1]?.lineNumber).toBe(3);
  });

  it("treats a file with no separator as a single block at line 0", () => {
    expect(scanTextRequests("GET https://a.dev/1\nAccept: */*\n", "http")).toEqual([
      { index: 0, name: "GET https://a.dev/1", method: "GET", url: "https://a.dev/1", kind: "http", lineNumber: 0 },
    ]);
  });

  it("returns nothing for an empty file", () => {
    expect(scanTextRequests("", "http")).toEqual([]);
    expect(scanTextRequests("\n\n", "http")).toEqual([]);
  });

  it("skips a header segment and a stray separator without spending an index", () => {
    const source = `@host = api.dev
# just a note

###

### Real
GET https://{{host}}/x
`;
    expect(scanTextRequests(source, "http")).toEqual([
      { index: 0, name: "Real", method: "GET", url: "https://{{host}}/x", kind: "http", lineNumber: 5 },
    ]);
  });

  it("marks a block GraphQL from the X-REQUEST-TYPE header", () => {
    const source = `### Search
POST https://a.dev/graphql
content-type: application/json
x-request-type: GraphQL

query { me { id } }

{ "q": "ada" }
`;
    expect(scanTextRequests(source, "http")[0]).toMatchObject({ kind: "graphql", method: "POST" });
  });

  it("does not read the marker out of the body", () => {
    const source = `### Plain
POST https://a.dev/x

X-REQUEST-TYPE: GraphQL
`;
    expect(scanTextRequests(source, "http")[0]?.kind).toBe("http");
  });

  it("defaults a method-less request line to GET and drops the HTTP version", () => {
    const blocks = scanTextRequests("### Bare\nhttps://a.dev/x HTTP/1.1\n", "http");
    expect(blocks[0]).toMatchObject({ method: "GET", url: "https://a.dev/x" });
  });

  it("joins an indented URL continuation", () => {
    const blocks = scanTextRequests("### Long\nGET https://a.dev/x?a=1\n  &b=2\n", "http");
    expect(blocks[0]?.url).toBe("https://a.dev/x?a=1&b=2");
  });

  it("skips @vars, comments and a pre-request script before the request line", () => {
    const source = `### Scripted
// a note
# another
@local = 1
< {%
pm.request.headers.add({ key: "x", value: "1" });
%}
DELETE https://a.dev/x
`;
    expect(scanTextRequests(source, "http")[0]).toMatchObject({
      method: "DELETE",
      url: "https://a.dev/x",
      lineNumber: 0,
    });
  });

  it("falls back to the # @name directive, then to the request line", () => {
    const source = `###
# @name Named
GET https://a.dev/1

###
GET https://a.dev/2
`;
    expect(scanTextRequests(source, "http").map((block) => block.name)).toEqual([
      "Named",
      "GET https://a.dev/2",
    ]);
  });

  it("reads a .grpc file as gRPC blocks", () => {
    const source = `### Say hello
{{grpcUrl}}/mock.v1.Mock/Say
proto: protos/mock.proto
x-trace: mandalo

{ "text": "hola" }

### Say again
{{grpcUrl}}/mock.v1.Mock/SayAgain
`;
    expect(scanTextRequests(source, "grpc")).toEqual([
      {
        index: 0,
        name: "Say hello",
        method: "POST",
        url: "{{grpcUrl}}",
        kind: "grpc",
        lineNumber: 0,
      },
      {
        index: 1,
        name: "Say again",
        method: "POST",
        url: "{{grpcUrl}}",
        kind: "grpc",
        lineNumber: 7,
      },
    ]);
  });

  it("names an unnamed gRPC block after its service and method, as the CLI does", () => {
    expect(scanTextRequests("{{u}}/p.S/M\n", "grpc")[0]?.name).toBe("p.S/M");
  });
});

describe("requestLenses", () => {
  it("puts Send and Send with env on every block's separator line", () => {
    expect(requestLenses(MULTI, "http", "auth/login.http")).toEqual([
      { line: 2, title: "▶ Send", command: "mandalo.sendRequest", relPath: "auth/login.http#0" },
      { line: 2, title: "▶ Send with env…", command: "mandalo.sendRequestWithEnv", relPath: "auth/login.http#0" },
      { line: 12, title: "▶ Send", command: "mandalo.sendRequest", relPath: "auth/login.http#1" },
      { line: 12, title: "▶ Send with env…", command: "mandalo.sendRequestWithEnv", relPath: "auth/login.http#1" },
    ]);
  });

  it("anchors a separator-less file on line 0", () => {
    expect(requestLenses("GET https://a.dev/1\n", "http", "ping.http").map((lens) => lens.line)).toEqual([
      0, 0,
    ]);
  });

  it("carries the gRPC identity", () => {
    const lenses = requestLenses("### A\n{{u}}/p.S/M\n", "grpc", "grpc/mock.grpc");
    expect(lenses.map((lens) => lens.relPath)).toEqual(["grpc/mock.grpc#0", "grpc/mock.grpc#0"]);
  });
});
