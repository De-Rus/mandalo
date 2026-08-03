import { describe, expect, it } from "vitest";
import {
  parseTextDocument,
  TextFormatError,
  withFileVars,
  type ParsedBlock,
} from "../../src/core/httpFormat";

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

  it("reads a CRLF file exactly like an LF one", () => {
    const source = "### R\nGET https://a.dev/x\nAccept: */*\n\nbody\n";
    expect(parseTextDocument("a.http", source.replace(/\n/g, "\r\n"), "http").blocks).toEqual(
      parseTextDocument("a.http", source, "http").blocks,
    );
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
      method: "GRPC",
      url: "{{grpcUrl}}/mock.v1.Mock/Say",
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
