import { describe, expect, it } from "vitest";
import { detectImportKind } from "./importKind";

describe("detectImportKind", () => {
  it("reads a Mándalo bundle first", () => {
    const detected = detectImportKind(
      '{"mandaloBundle":2,"openapi":"3.0.0","info":{},"item":[]}',
    );
    expect(detected.kind).toBe("bundle");
    expect(detected.confident).toBe(true);
  });

  it("reads a JSON OpenAPI document", () => {
    expect(detectImportKind('{"openapi":"3.1.0","paths":{}}').kind).toBe(
      "openapi",
    );
    expect(detectImportKind('{"swagger":"2.0","paths":{}}').kind).toBe(
      "openapi",
    );
  });

  it("reads a YAML OpenAPI document", () => {
    expect(detectImportKind("openapi: 3.1.0\npaths: {}\n").kind).toBe("openapi");
    expect(detectImportKind("swagger: '2.0'\npaths: {}\n").kind).toBe("openapi");
  });

  it("does not call a description that mentions openapi a specification", () => {
    const detected = detectImportKind(
      '{"info":{"name":"talks about \\"openapi\\" a lot"},"item":[]}',
    );
    expect(detected.kind).toBe("postman");
    expect(detected.confident).toBe(true);
  });

  it("reads a Postman collection by its info and item pair", () => {
    expect(
      detectImportKind('{"info":{"name":"Coll"},"item":[]}').confident,
    ).toBe(true);
    expect(detectImportKind('{"_postman_id":"abc"}').kind).toBe("postman");
  });

  it("says so when nothing names the format", () => {
    const detected = detectImportKind('{"hello":"world"}');
    expect(detected.kind).toBe("postman");
    expect(detected.confident).toBe(false);
    expect(detected.reason).toMatch(/pick another importer/i);
  });

  it("does not throw on a document that is not JSON at all", () => {
    expect(detectImportKind("not json").kind).toBe("postman");
  });
});
