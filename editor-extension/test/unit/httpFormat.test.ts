import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const REPO_ROOT_FOR_FIXTURES = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
import {
  parseTextDocument,
  TextFormatError,
  withFileVars,
  type ParsedBlock,
} from "../../../src/lib/format/httpFormat";
import { renderRequest } from "../../../src/lib/format/render";

function one(source: string, kind: "http" | "grpc" = "http"): ParsedBlock {
  const block = parseTextDocument("auth/login.http", source, kind).blocks[0];
  if (block === undefined) throw new Error("expected one block");
  return block;
}

describe("parseTextDocument", () => {
  it("reads a request line, headers and a body", () => {
    const model = one(`### Login
POST https://a.dev/auth/login
Content-Type: application/json

{ "user": "ada" }
`).model;
    expect(model).toMatchObject({
      id: "auth-login-http-0",
      name: "Login",
      kind: "http",
      method: "POST",
      url: "https://a.dev/auth/login",
      headers: [["Content-Type", "application/json"]],
      body: '{ "user": "ada" }',
      tests: [],
      captures: [],
    });
  });

  it("indexes every block in the file", () => {
    const blocks = parseTextDocument(
      "api.http",
      "### One\nGET https://a.dev/1\n\n### Two\nGET https://a.dev/2\n",
      "http",
    ).blocks;
    expect(blocks.map((block) => [block.index, block.name, block.model.id])).toEqual([
      [0, "One", "api-http-0"],
      [1, "Two", "api-http-1"],
    ]);
  });

  it("reads Authorization back as typed auth and drops the header", () => {
    const model = one("### R\nGET https://a.dev/x\nAuthorization: Bearer {{token}}\n").model;
    expect(model.auth).toEqual({ type: "bearer", token: "{{token}}" });
    expect(model.headers).toEqual([]);
  });

  it("reads basic auth as a username and a password", () => {
    const model = one("### R\nGET https://a.dev/x\nAuthorization: Basic ada:lovelace\n").model;
    expect(model.auth).toEqual({ type: "basic", username: "ada", password: "lovelace" });
  });

  it("leaves an api key header as a header, because that is what it is", () => {
    const model = one("### R\nGET https://a.dev/x\nx-api-key: k\n").model;
    expect(model.auth).toEqual({ type: "none" });
    expect(model.headers).toEqual([["x-api-key", "k"]]);
  });

  it("splits a GraphQL document from its variables and eats the marker header", () => {
    const model = one(`### Search
POST https://a.dev/graphql
X-REQUEST-TYPE: GraphQL

query Search($q: String!) { users(q: $q) { id } }

{ "q": "ada" }
`).model;
    expect(model.kind).toBe("graphql");
    expect(model.headers).toEqual([]);
    expect(model.graphql).toEqual({
      query: "query Search($q: String!) { users(q: $q) { id } }",
      variables: '{ "q": "ada" }',
    });
  });

  it("sends no variables when the GraphQL body has no object block", () => {
    const model = one(
      "### G\nPOST https://a.dev/graphql\nX-REQUEST-TYPE: GraphQL\n\n{ users { id } }\n",
    ).model;
    expect(model.graphql).toEqual({ query: "{ users { id } }", variables: "" });
  });

  it("keeps both scripts, dedented", () => {
    const model = one(`### R
< {%
  pm.variables.set("a", 1);
%}
GET https://a.dev/x

> {%
  pm.test("ok", function () {});
%}
`).model;
    expect(model.scripts.pre).toBe('pm.variables.set("a", 1);');
    expect(model.scripts.post).toBe('pm.test("ok", function () {});');
  });

  it("records a `< file` body as a path, never as body text", () => {
    const model = one("### R\nPOST https://a.dev/x\n\n< ./files/payload.json\n").model;
    expect(model.bodyFile).toBe("files/payload.json");
    expect(model.body).toBeUndefined();
  });

  it("keeps an XML body a body, because `<` only opens a file with whitespace after it", () => {
    const model = one("### R\nPOST https://a.dev/x\n\n<note>hi</note>\n").model;
    expect(model.body).toBe("<note>hi</note>");
    expect(model.bodyFile).toBeUndefined();
  });

  it("falls back to the `# @name` directive, then to the request line", () => {
    const blocks = parseTextDocument(
      "api.http",
      "###\n# @name Named\nGET https://a.dev/1\n\n###\nGET https://a.dev/2\n",
      "http",
    ).blocks;
    expect(blocks.map((block) => block.name)).toEqual(["Named", "GET https://a.dev/2"]);
  });

  it("skips a file header and a stray separator without spending an index", () => {
    const document = parseTextDocument(
      "api.http",
      "@host = api.dev\n# a note\n\n###\n\n### Real\nGET https://{{host}}/x\n",
      "http",
    );
    expect(document.vars).toEqual([["host", "api.dev"]]);
    expect(document.blocks.map((block) => [block.index, block.name])).toEqual([[0, "Real"]]);
  });

  it("reads a CRLF file exactly like an LF one, apart from the bytes of its body", () => {
    const source = "### R\nGET https://a.dev/x\nAccept: */*\n\nbody\n";
    const crlf = parseTextDocument("a.http", source.replace(/\n/g, "\r\n"), "http").blocks;
    const lf = parseTextDocument("a.http", source, "http").blocks;
    expect(crlf.map((block) => ({ ...block.model, body: undefined }))).toEqual(
      lf.map((block) => ({ ...block.model, body: undefined })),
    );
  });

  // A body is a slice of the file, so a CRLF file puts CRLF on the wire — the Rust
  // reader spans the same bytes and the two engines must not disagree about them.
  it("keeps a CRLF body's own line endings", () => {
    const crlf = one("### R\r\nPOST https://a.dev/x\r\n\r\n{\r\n  \"a\": 1\r\n}\r\n");
    expect(crlf.model.body).toBe('{\r\n  "a": 1\r\n}');
    const lf = one('### R\nPOST https://a.dev/x\n\n{\n  "a": 1\n}\n');
    expect(lf.model.body).toBe('{\n  "a": 1\n}');
  });

  it("joins an indented URL continuation and drops a trailing HTTP version", () => {
    expect(one("### R\nGET https://a.dev/x?a=1\n  &b=2\n").model.url).toBe("https://a.dev/x?a=1&b=2");
    expect(one("### R\nhttps://a.dev/x HTTP/1.1\n").model).toMatchObject({
      method: "GET",
      url: "https://a.dev/x",
    });
  });
});

describe("what parseTextDocument refuses", () => {
  const cases: [string, string, "http" | "grpc"][] = [
    ["### R\nFETCH https://a.dev/x\n", '"FETCH" is not an HTTP method', "http"],
    ["### R\nGET HTTP/1.1\n", "this request line has no URL", "http"],
    ["### R\n", "this block has no request line", "http"],
    ["### R\nGET https://a.dev/x\nAccept json\n", "expected `Name: value`", "http"],
    ["### R\nGET https://a.dev/x\nBad Header: x\n", "is not a valid header name", "http"],
    ["### R\nGET https://a.dev/x\n\n> {%\n", "never closed with `%}`", "http"],
    ["### R\nGET https://a.dev/x\n\n< /etc/passwd\n", "workspace-relative path", "http"],
    ["### R\nGET https://a.dev/x\n\n< ../../secret.json\n", "stay inside the workspace", "http"],
    ["### R\nGET https://a.dev/x\n\n<@ ./a.json\n", "does not support `<@` file bodies", "http"],
    [
      "### R\nPOST https://a.dev/x\nX-REQUEST-TYPE: grpc\n",
      "only marks a GraphQL request",
      "http",
    ],
    ["# @timeout 30\n### R\nGET https://a.dev/x\n", "does not support the `@timeout` directive", "http"],
    ["@my var = x\n### R\nGET https://a.dev/x\n", "is not a valid variable name", "http"],
    ["### S\n{{grpcUrl}}\n", "expected `target/package.Service/Method`", "grpc"],
    ["### S\nmock.v1.Mock/Say\n", "names no target", "grpc"],
    ["### S\nlocalhost:1//Say\n", "this call line has no service", "grpc"],
    ["### S\nlocalhost:1/p.S/M\ngrpc-timeout: 1\n", "reserves the `grpc-` metadata prefix", "grpc"],
    ["### S\nlocalhost:1/p.S/M\nproto: /tmp/x.proto\n", "workspace-relative path", "grpc"],
  ];

  for (const [source, message, kind] of cases) {
    it(`says: ${message}`, () => {
      expect(() => parseTextDocument("a", source, kind)).toThrow(TextFormatError);
      expect(() => parseTextDocument("a", source, kind)).toThrow(message);
    });
  }

  it("names the line the problem is on", () => {
    try {
      parseTextDocument("a", "### R\nGET https://a.dev/x\nAccept json\n", "http");
      expect.unreachable();
    } catch (error) {
      expect((error as TextFormatError).line).toBe(3);
      expect((error as Error).message).toMatch(/^line 3: /);
    }
  });
});

describe("gRPC blocks", () => {
  it("splits the call line into target, service and method", () => {
    const model = one(
      '### Say\n{{grpcUrl}}/mock.v1.Mock/Say\nproto: protos/mock.proto\nx-trace: mandalo\n\n{"text": "hola"}\n',
      "grpc",
    ).model;
    expect(model).toMatchObject({
      kind: "grpc",
      method: "POST",
      url: "{{grpcUrl}}",
      grpc: {
        protoPaths: ["protos/mock.proto"],
        service: "mock.v1.Mock",
        method: "Say",
        message: '{"text": "hola"}',
        metadata: [["x-trace", "mandalo"]],
      },
    });
  });

  it("sends {} when a call carries no message", () => {
    expect(one("### S\nlocalhost:1/p.S/M\n", "grpc").model.grpc?.message).toBe("{}");
  });

  it("collects every repeated proto path", () => {
    const model = one("### S\nlocalhost:1/p.S/M\nproto: a.proto\nproto: b.proto\n", "grpc").model;
    expect(model.grpc?.protoPaths).toEqual(["a.proto", "b.proto"]);
  });
});

describe("withFileVars", () => {
  it("applies the file's own @vars, leaving the rest for the environment", () => {
    const document = parseTextDocument(
      "a.http",
      "@host = api.dev\n\n### R\nGET https://{{host}}/x\nAuthorization: Bearer {{token}}\n",
      "http",
    );
    const model = withFileVars(document)[0]!.model;
    expect(model.url).toBe("https://api.dev/x");
    expect(model.auth).toEqual({ type: "bearer", token: "{{token}}" });
  });

  it("resolves @vars in declaration order, each against the ones before it", () => {
    const document = parseTextDocument(
      "a.http",
      "@host = api.dev\n@base = https://{{host}}/v1\n\n### R\nGET {{base}}/x\n",
      "http",
    );
    expect(withFileVars(document)[0]!.model.url).toBe("https://api.dev/v1/x");
  });
});

describe("multipart form-data", () => {
  const MULTIPART = `### Upload avatar
POST {{baseUrl}}/body/multipart
Content-Type: multipart/form-data; boundary=WebAppBoundary

--WebAppBoundary
Content-Disposition: form-data; name="title"

Avatar shot
--WebAppBoundary
Content-Disposition: form-data; name="photo"; filename="a.png"
Content-Type: image/png

< files/a.png
--WebAppBoundary--
`;

  it("parses parts into formdata rows and folds the content type away", () => {
    const model = parseTextDocument("upload", MULTIPART, "http").blocks[0]!.model;
    expect(model.formdata).toEqual([
      { key: "title", value: "Avatar shot" },
      { key: "photo", files: ["files/a.png"], contentType: "image/png" },
    ]);
    expect(model.body).toBeUndefined();
    expect(model.headers.some(([k]) => k.toLowerCase() === "content-type")).toBe(false);
  });

  it("folds repeated names into one field with several files", () => {
    const raw = `### U
POST https://a.dev/x
Content-Type: multipart/form-data; boundary=B

--B
Content-Disposition: form-data; name="photos"; filename="a.png"

< files/a.png
--B
Content-Disposition: form-data; name="photos"; filename="b.png"

< files/b.png
--B--
`;
    const model = parseTextDocument("u", raw, "http").blocks[0]!.model;
    expect(model.formdata).toEqual([
      { key: "photos", files: ["files/a.png", "files/b.png"] },
    ]);
  });

  it("refuses a multipart content type without a boundary", () => {
    const raw = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\n--x\n";
    expect(() => parseTextDocument("u", raw, "http")).toThrow(/boundary/);
  });

  it("refuses an unclosed multipart body", () => {
    const raw =
      '### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name="a"\n\n1\n';
    expect(() => parseTextDocument("u", raw, "http")).toThrow(/never closed/);
  });

  it("refuses inline file bytes in a file part", () => {
    const raw =
      '### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name="f"; filename="a.png"\n\nPNG\n--B--\n';
    expect(() => parseTextDocument("u", raw, "http")).toThrow(/< path/);
  });

  it("renders formdata back to a parseable multipart block", () => {
    const model = parseTextDocument("upload", MULTIPART, "http").blocks[0]!.model;
    const rendered = renderRequest(model, "\n");
    const reread = parseTextDocument("upload", rendered, "http").blocks[0]!.model;
    expect(reread.formdata).toEqual(model.formdata);
  });

  const FORM_FIELDS = `### Upload avatar
POST {{baseUrl}}/body/multipart
Content-Type: multipart/form-data

title = Avatar shot
photo = < files/a.png; type=image/png
`;

  it("reads the field-per-line form into the same rows as the boundary form", () => {
    const readable = parseTextDocument("upload", FORM_FIELDS, "http").blocks[0]!.model;
    const literal = parseTextDocument("upload", MULTIPART, "http").blocks[0]!.model;
    expect(readable.formdata).toEqual(literal.formdata);
    expect(readable.formdata).toEqual([
      { key: "title", value: "Avatar shot" },
      { key: "photo", files: ["files/a.png"], contentType: "image/png" },
    ]);
    expect(readable.headers.some(([k]) => k.toLowerCase() === "content-type")).toBe(false);
  });

  it("round-trips the field-per-line form byte for byte", () => {
    const model = parseTextDocument("upload", FORM_FIELDS, "http").blocks[0]!.model;
    expect(renderRequest(model, "\n")).toBe(FORM_FIELDS);
  });

  it("repeats a name to put several files on one field, in order", () => {
    const raw = `### U
POST https://a.dev/x
Content-Type: multipart/form-data

attachments = < files/a.txt
attachments = < files/b.txt
attachments = < files/c.txt
`;
    const model = parseTextDocument("u", raw, "http").blocks[0]!.model;
    expect(model.formdata).toEqual([
      { key: "attachments", files: ["files/a.txt", "files/b.txt", "files/c.txt"] },
    ]);
    expect(renderRequest(model, "\n")).toBe(`### U
POST https://a.dev/x
Content-Type: multipart/form-data

attachments = < files/a.txt < files/b.txt < files/c.txt
`);
  });

  it("puts several files on one field line", () => {
    const raw = `### U
POST https://a.dev/x
Content-Type: multipart/form-data

attachments = < ./files/a.txt < ./files/b.txt <./files/c.txt; type=text/plain
`;
    const model = parseTextDocument("u", raw, "http").blocks[0]!.model;
    expect(model.formdata).toEqual([
      {
        key: "attachments",
        files: ["files/a.txt", "files/b.txt", "files/c.txt"],
        contentType: "text/plain",
      },
    ]);
    expect(renderRequest(model, "\n")).toBe(`### U
POST https://a.dev/x
Content-Type: multipart/form-data

attachments = < files/a.txt < files/b.txt < files/c.txt; type=text/plain
`);
  });

  it("still reads the older `name < ./path` spelling, and rewrites it", () => {
    const legacy = FORM_FIELDS.replace("photo = < files", "photo < files");
    const model = parseTextDocument("upload", legacy, "http").blocks[0]!.model;
    expect(model.formdata).toEqual(
      parseTextDocument("upload", FORM_FIELDS, "http").blocks[0]!.model.formdata,
    );
    expect(renderRequest(model, "\n")).toBe(FORM_FIELDS);
  });

  it("is forgiving about whitespace, and leaves markup-looking values alone", () => {
    const raw = `### U
POST https://a.dev/x
Content-Type: multipart/form-data

title=Avatar shot
photo=<./files/a.png;type=image/png
bio = <b>bold</b>
`;
    const model = parseTextDocument("u", raw, "http").blocks[0]!.model;
    expect(model.formdata).toEqual([
      { key: "title", value: "Avatar shot" },
      { key: "photo", files: ["files/a.png"], contentType: "image/png" },
      { key: "bio", value: "<b>bold</b>" },
    ]);
  });

  it("refuses a text value that would read back as a file reference", () => {
    const model = parseTextDocument("upload", FORM_FIELDS, "http").blocks[0]!.model;
    model.formdata = [{ key: "note", value: "< ./files/a.png" }];
    expect(() => renderRequest(model, "\n")).toThrow(/file reference/);
  });

  it("refuses a boundary parameter on a field-per-line body", () => {
    const raw =
      "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\ntitle = hola\n";
    expect(() => parseTextDocument("u", raw, "http")).toThrow(/remove the `boundary=` parameter/);
  });

  it("refuses a field with no separator, no name, or an unknown parameter", () => {
    const body = (line: string) =>
      `### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\n${line}\n`;
    expect(() => parseTextDocument("u", body("just words"), "http")).toThrow(
      /a form field reads `name = value`, or `name = < \.\/path`/,
    );
    expect(() => parseTextDocument("u", body(" = orphan"), "http")).toThrow(/no name/);
    expect(() => parseTextDocument("u", body("p = < a.png; charset=utf-8"), "http")).toThrow(
      /`; type=…`/,
    );
  });

  it("refuses a field-per-line path that leaves the workspace", () => {
    for (const path of ["/etc/passwd", "../../secret.env", "C:/keys.pem"]) {
      const raw = `### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\nleak = < ${path}\n`;
      expect(() => parseTextDocument("u", raw, "http")).toThrow(/workspace/);
    }
  });

  it("substitutes file-scoped vars into a field-per-line path", () => {
    const raw = `@dir = files
### U
POST https://a.dev/x
Content-Type: multipart/form-data

photo = < {{dir}}/a.png
`;
    const model = withFileVars(parseTextDocument("u", raw, "http"))[0]!.model;
    expect(model.formdata?.[0]?.files).toEqual(["files/a.png"]);
  });

  it("substitutes file-scoped vars into formdata rows", () => {
    const raw = `@dir = files
### U
POST https://a.dev/x
Content-Type: multipart/form-data; boundary=B

--B
Content-Disposition: form-data; name="photo"; filename="a.png"

< {{dir}}/a.png
--B--
`;
    const document = parseTextDocument("u", raw, "http");
    const model = withFileVars(document)[0]!.model;
    expect(model.formdata?.[0]?.files).toEqual(["files/a.png"]);
  });
});

describe("multipart diagnostics", () => {
  it("says a part is missing its blank line instead of blaming the header", () => {
    const raw =
      '### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name="a"\n--B--\n';
    expect(() => parseTextDocument("u", raw, "http")).toThrow(/blank line/);
  });

  it("points the squiggle at the offending line inside the body", () => {
    const raw =
      '### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name="a"\nBogus part header\n\n1\n--B--\n';
    try {
      parseTextDocument("u", raw, "http");
      throw new Error("expected the parse to fail");
    } catch (error) {
      if (!(error instanceof TextFormatError)) throw error;
      expect(raw.slice(error.offset, error.offset + error.length)).toBe("Bogus part header");
      expect(error.line).toBe(7);
    }
  });
});

describe("stream directives", () => {
  it("reads `@reconnect off` the way the Rust parser does", () => {
    const raw = "### SSE\n# @reconnect off\nhttps://a.dev/sse\nAccept: text/event-stream\n";
    const model = parseTextDocument("s", raw, "http").blocks[0]!.model;
    expect(model.stream).toEqual({ autoReconnect: false });
  });

  it("treats a bare `@reconnect` as on, and leaves it unset when absent", () => {
    const on = parseTextDocument("s", "### A\n# @reconnect\nhttps://a.dev/sse\n", "http");
    expect(on.blocks[0]!.model.stream).toEqual({ autoReconnect: true });
    const none = parseTextDocument("s", "### A\nhttps://a.dev/sse\n", "http");
    expect(none.blocks[0]!.model.stream).toBeUndefined();
  });

  it("refuses a value that is neither on nor off", () => {
    expect(() =>
      parseTextDocument("s", "### A\n# @reconnect sometimes\nhttps://a.dev/sse\n", "http"),
    ).toThrow(/on or off/);
  });

  it("parses the example workspace's streaming requests", () => {
    const raw = readFileSync(
      join(REPO_ROOT_FOR_FIXTURES, "examples/mock-workspace/collections/mock/streams/events.http"),
      "utf8",
    );
    expect(parseTextDocument("events", raw, "http").blocks).toHaveLength(2);
  });
});

describe("inherited auth directive", () => {
  const RAW = "### Me\n# @auth inherited\nGET https://a.dev/me\nAuthorization: Bearer {{token}}\n";

  it("reads `# @auth inherited` the way the Rust parser does", () => {
    const model = parseTextDocument("m", RAW, "http").blocks[0]!.model;
    expect(model.auth).toEqual({
      type: "inherited",
      auth: { type: "bearer", token: "{{token}}" },
    });
    expect(model.headers).toEqual([]);
  });

  it("writes the directive back, so a file the writer produced reads back the same", () => {
    const model = parseTextDocument("m", RAW, "http").blocks[0]!.model;
    const rendered = renderRequest(model, "\n");
    expect(rendered).toContain("# @auth inherited");
    expect(parseTextDocument("m", rendered, "http").blocks[0]!.model.auth).toEqual(model.auth);
  });

  it("refuses a value other than inherited", () => {
    expect(() =>
      parseTextDocument("m", "### A\n# @auth bearer\nGET https://a.dev/x\n", "http"),
    ).toThrow(/only takes `inherited`/);
  });

  it("leaves auth alone when the directive is absent", () => {
    const raw = "### A\nGET https://a.dev/x\nAuthorization: Bearer t\n";
    expect(parseTextDocument("a", raw, "http").blocks[0]!.model.auth).toEqual({
      type: "bearer",
      token: "t",
    });
  });
});
