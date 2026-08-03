import { describe, expect, it } from "vitest";
import type { EnvironmentView } from "./api";
import { newDraft } from "./draft";
import {
  describeVar,
  draftVarNames,
  previewResolve,
  splitVars,
  varTone,
} from "./vars";

const vars = { baseUrl: "https://api.dev", userId: "7" };

describe("variable segmentation", () => {
  it("splits literal text from variable tokens", () => {
    const segments = splitVars("{{baseUrl}}/users/{{userId}}", vars);
    expect(segments.map((s) => s.text)).toEqual([
      "{{baseUrl}}",
      "/users/",
      "{{userId}}",
    ]);
    expect(segments[0].resolved).toBe("https://api.dev");
    expect(segments[1].name).toBeNull();
  });

  it("marks unknown variables as unresolved", () => {
    const [segment] = splitVars("{{nope}}", vars);
    expect(segment.name).toBe("nope");
    expect(segment.resolved).toBeNull();
  });

  it("tolerates padding inside the braces", () => {
    expect(splitVars("{{ baseUrl }}", vars)[0].resolved).toBe("https://api.dev");
  });
});

const env: EnvironmentView = {
  name: "hosted",
  vars: {
    baseUrl: { secret: false, value: "https://api.dev", set: true },
    apiKey: { secret: true, value: null, hosts: [], set: true },
    adminKey: { secret: true, value: null, hosts: [], set: false },
  },
};

describe("variable description", () => {
  it("reports a plain value and where it came from", () => {
    expect(describeVar("baseUrl", env)).toEqual({
      name: "baseUrl",
      state: "value",
      value: "https://api.dev",
      env: "hosted",
      secretSet: false,
    });
  });

  it("never carries a secret value, only whether this machine has one", () => {
    expect(describeVar("apiKey", env)).toEqual({
      name: "apiKey",
      state: "secret",
      value: null,
      env: "hosted",
      secretSet: true,
    });
    expect(describeVar("adminKey", env).secretSet).toBe(false);
  });

  it("marks an unknown name missing and names the environment it is missing from", () => {
    const described = describeVar("nope", env);
    expect(described.state).toBe("missing");
    expect(described.env).toBe("hosted");
  });

  it("treats {{$dynamic}} names as generated per run", () => {
    expect(describeVar("$guid", env).state).toBe("dynamic");
  });

  it("falls back to the resolved map when no environment is loaded", () => {
    expect(describeVar("baseUrl", null, { baseUrl: "http://local" })).toEqual({
      name: "baseUrl",
      state: "value",
      value: "http://local",
      env: null,
      secretSet: false,
    });
  });

  it("gives unresolved variables the existing red tone", () => {
    expect(varTone("missing")).toBe("var-bad");
    expect(varTone("value")).toBe("var-ok");
    expect(varTone("secret")).toBe("var-secret");
  });
});

describe("draft variable collection", () => {
  it("collects the variables of every field the request kind uses", () => {
    const draft = newDraft("Sample", "http");
    draft.url = "{{baseUrl}}/users";
    draft.headers = [
      { id: "1", key: "authorization", value: "Bearer {{apiKey}}", enabled: true },
      { id: "2", key: "x-off", value: "{{ignored}}", enabled: false },
    ];
    draft.body = '{"tenant": "{{tenant}}"}';
    draft.graphqlQuery = "{{notForHttp}}";
    expect(draftVarNames(draft)).toEqual(["baseUrl", "apiKey", "tenant"]);
  });

  it("reads the gRPC fields for a gRPC request", () => {
    const draft = newDraft("Sample", "grpc");
    draft.url = "{{grpcUrl}}";
    draft.grpc.message = '{"id": "{{userId}}"}';
    expect(draftVarNames(draft)).toEqual(["grpcUrl", "userId"]);
  });
});

describe("preview resolution", () => {
  it("substitutes what it knows and leaves the rest visible", () => {
    expect(previewResolve("{{baseUrl}}/{{nope}}", vars)).toBe(
      "https://api.dev/{{nope}}",
    );
  });
});
