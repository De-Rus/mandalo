import { describe, expect, it } from "vitest";
import {
  ParseError,
  parseCollectionManifest,
  parseEnvironment,
  parseRequest,
  parseWorkspaceManifest,
  renderRequest,
} from "../../src/core/parse";

const FULL = `
schema_version = 1
id = "550e8400-e29b-41d4-a716-446655440000"
name = "Login"
kind = "http"
method = "POST"
url = "{{base}}/auth/login"
headers = [["Content-Type", "application/json"], ["X-Trace", "1"]]
body = "{\\"user\\":\\"me\\"}"

[auth]
type = "bearer"
token = "{{token}}"

[scripts]
pre = "pm.environment.set('ts', Date.now())"
post = "pm.test('ok', () => pm.response.to.have.status(200))"

[[tests]]
kind = "status"
op = "eq"
value = 200

[[tests]]
kind = "header"
op = "exists"
name = "X-Request-Id"

[[captures]]
from = "body.$.token"
into = "token"
scope = "run"
`;

describe("parseRequest", () => {
  it("reads every section of a full request", () => {
    const model = parseRequest(FULL);
    expect(model.id).toBe("550e8400-e29b-41d4-a716-446655440000");
    expect(model.name).toBe("Login");
    expect(model.kind).toBe("http");
    expect(model.method).toBe("POST");
    expect(model.url).toBe("{{base}}/auth/login");
    expect(model.headers).toEqual([
      ["Content-Type", "application/json"],
      ["X-Trace", "1"],
    ]);
    expect(model.body).toBe('{"user":"me"}');
    expect(model.auth).toEqual({ type: "bearer", token: "{{token}}" });
    expect(model.scripts.pre).toContain("pm.environment.set");
    expect(model.scripts.post).toContain("pm.test");
    expect(model.tests).toHaveLength(2);
    expect(model.tests[0]).toMatchObject({ kind: "status", op: "eq", value: 200 });
    expect(model.tests[1]).toMatchObject({ kind: "header", op: "exists", name: "X-Request-Id" });
    expect(model.captures).toEqual([{ from: "body.$.token", into: "token", scope: "run" }]);
  });

  it("round-trips through renderRequest", () => {
    const model = parseRequest(FULL);
    const again = parseRequest(renderRequest(model));
    expect(again).toEqual(model);
  });

  it("round-trips a graphql request", () => {
    const model = parseRequest(`
name = "Search"
kind = "graphql"
method = "POST"
url = "{{base}}/graphql"

[graphql]
query = "query { me { id } }"
variables = "{}"
`);
    expect(model.graphql).toEqual({ query: "query { me { id } }", variables: "{}" });
    expect(parseRequest(renderRequest(model))).toEqual(model);
  });

  it("round-trips a grpc request and accepts both proto path spellings", () => {
    const snake = parseRequest(`
name = "Say"
kind = "grpc"
url = "localhost:50051"

[grpc]
proto_paths = ["a.proto"]
service = "pkg.Svc"
method = "Say"
message = "{}"
metadata = [["k", "v"]]
`);
    expect(snake.grpc?.protoPaths).toEqual(["a.proto"]);
    expect(snake.grpc?.metadata).toEqual([["k", "v"]]);
    expect(parseRequest(renderRequest(snake))).toEqual(snake);

    const camel = parseRequest(`
name = "Say"
kind = "grpc"
url = "localhost:50051"

[grpc]
protoPaths = ["a.proto"]
service = "pkg.Svc"
method = "Say"
`);
    expect(camel.grpc?.protoPaths).toEqual(["a.proto"]);
  });

  it("round-trips every auth flavour", () => {
    for (const auth of [
      '[auth]\ntype = "none"',
      '[auth]\ntype = "basic"\nusername = "u"\npassword = "p"',
      '[auth]\ntype = "apikey"\nkey = "k"\nvalue = "v"\nplacement = "query"',
    ]) {
      const model = parseRequest(`name = "R"\nurl = "http://x"\n${auth}`);
      expect(parseRequest(renderRequest(model)).auth).toEqual(model.auth);
    }
  });

  it("defaults kind to http and method to GET", () => {
    const model = parseRequest('name = "R"\nurl = "http://x"');
    expect(model.kind).toBe("http");
    expect(model.method).toBe("GET");
  });

  it("fails loud on missing url", () => {
    expect(() => parseRequest('name = "R"')).toThrow(ParseError);
    expect(() => parseRequest('name = "R"')).toThrow(/missing required key "url"/);
  });

  it("fails loud on malformed TOML", () => {
    expect(() => parseRequest("name = ")).toThrow(/invalid TOML/);
  });

  it("fails loud on a non-string header pair", () => {
    expect(() => parseRequest('name = "R"\nurl = "u"\nheaders = [["a", 1]]')).toThrow(/must hold strings/);
  });

  it("fails loud on an unknown auth type", () => {
    expect(() => parseRequest('name = "R"\nurl = "u"\n[auth]\ntype = "oauth9"')).toThrow(/\[auth\].type/);
  });

  it("fails loud when a test entry has no op", () => {
    expect(() => parseRequest('name = "R"\nurl = "u"\n[[tests]]\nkind = "status"')).toThrow(
      /missing required key "op"/,
    );
  });
});

describe("manifests and environments", () => {
  it("reads a collection manifest", () => {
    expect(parseCollectionManifest('schema_version = 1\nid = "x"\nname = "Acme"')).toEqual({
      schemaVersion: 1,
      id: "x",
      name: "Acme",
    });
  });

  it("reads a workspace manifest", () => {
    expect(parseWorkspaceManifest('schema_version = 1\nid = "w"\nname = "Personal"')).toEqual({
      schemaVersion: 1,
      id: "w",
      name: "Personal",
    });
  });

  it("reads an environment and falls back to the file name", () => {
    expect(parseEnvironment('[vars]\nbase = "https://x"', "staging")).toEqual({
      name: "staging",
      vars: { base: "https://x" },
    });
    expect(parseEnvironment('name = "prod"\n[vars]\nbase = "https://x"', "ignored").name).toBe("prod");
  });

  it("fails loud on a non-string variable", () => {
    expect(() => parseEnvironment("[vars]\nport = 8080", "local")).toThrow(/must be a string/);
  });
});
