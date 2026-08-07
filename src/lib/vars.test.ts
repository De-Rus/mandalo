import { describe, expect, it } from "vitest";
import type { EnvironmentView } from "./api";
import { newDraft } from "./draft";
import {
  describeVar,
  draftVarNames,
  previewResolve,
  splitVars,
  unresolvedVars,
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
    baseUrl: {
      shared: true,
      secret: false,
      value: "https://api.dev",
      set: true,
      source: "file",
    },
    apiKey: {
      shared: false,
      secret: true,
      value: null,
      hosts: ["api.dev"],
      set: true,
      source: "local",
    },
    adminKey: { shared: false, secret: true, value: null, hosts: [], set: false },
    devUrl: {
      shared: false,
      secret: false,
      value: null,
      hosts: [],
      set: true,
      source: "local",
    },
    unsetLocal: { shared: false, secret: false, value: null, hosts: [], set: false },
  },
};

describe("variable description", () => {
  it("reports a shared value and where it came from", () => {
    expect(describeVar("baseUrl", env)).toEqual({
      name: "baseUrl",
      state: "value",
      value: "https://api.dev",
      env: "hosted",
      held: true,
      source: "file",
      hosts: [],
    });
  });

  it("never carries a secret value, only whether a machine holds one", () => {
    expect(describeVar("apiKey", env)).toEqual({
      name: "apiKey",
      state: "secret",
      value: null,
      env: "hosted",
      held: true,
      source: "local",
      hosts: ["api.dev"],
    });
    expect(describeVar("adminKey", env).held).toBe(false);
  });

  it("separates a local variable from a secret one", () => {
    const local = describeVar("devUrl", env);
    expect(local.state).toBe("local");
    expect(local.held).toBe(true);
    expect(local.source).toBe("local");
    expect(local.value).toBeNull();
  });

  it("drops a secret value the backend should never have sent", () => {
    const leaky: EnvironmentView = {
      name: "hosted",
      vars: {
        token: {
          shared: false,
          secret: true,
          value: "leaked",
          hosts: [],
          set: true,
        },
      },
    };
    expect(describeVar("token", leaky).value).toBeNull();
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
      held: true,
      source: null,
      hosts: [],
    });
  });

  it("gives unresolved variables the existing red tone", () => {
    expect(varTone("missing")).toBe("var-bad");
    expect(varTone("value")).toBe("var-ok");
    expect(varTone("secret")).toBe("var-secret");
    expect(varTone("local")).toBe("var-local");
  });
});

describe("unresolved variables", () => {
  it("is empty when everything resolves", () => {
    expect(unresolvedVars(["baseUrl", "apiKey", "devUrl", "$guid"], env)).toEqual(
      [],
    );
  });

  it("catches an undefined name, an unset secret and an unset local alike", () => {
    expect(
      unresolvedVars(["nope", "adminKey", "unsetLocal"], env).map((d) => [
        d.name,
        d.state,
      ]),
    ).toEqual([
      ["nope", "missing"],
      ["adminKey", "secret"],
      ["unsetLocal", "local"],
    ]);
  });

  it("treats a name the resolved map already carries as resolved", () => {
    expect(unresolvedVars(["late"], null, { late: "x" })).toEqual([]);
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
