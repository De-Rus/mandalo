import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import worker from "./src/index.js";

const contract = JSON.parse(
  readFileSync(fileURLToPath(new URL("../contract.json", import.meta.url)), "utf8"),
);

const BASE = "https://api.mandalo.dev";

function contains(actual, expected) {
  if (Array.isArray(expected)) {
    return (
      Array.isArray(actual) &&
      expected.length <= actual.length &&
      expected.every((value, index) => contains(actual[index], value))
    );
  }
  if (expected && typeof expected === "object") {
    return (
      actual &&
      typeof actual === "object" &&
      Object.entries(expected).every(([key, value]) => contains(actual[key], value))
    );
  }
  return actual === expected;
}

for (const testCase of contract.cases) {
  test(`worker: ${testCase.name}`, async () => {
    const request = new Request(`${BASE}${testCase.path}`, {
      method: testCase.method,
      headers: testCase.headers ?? {},
      body: testCase.body,
    });
    const response = await worker.fetch(request);
    const body = await response.text();

    assert.equal(response.status, testCase.status, `body was ${body}`);

    if (testCase.json !== undefined) {
      const parsed = JSON.parse(body);
      assert.ok(
        contains(parsed, testCase.json),
        `${JSON.stringify(parsed)} lacks ${JSON.stringify(testCase.json)}`,
      );
    }
    if (testCase.bodyContains) assert.ok(body.includes(testCase.bodyContains), body);
    if (testCase.bodyEquals !== undefined) assert.equal(body, testCase.bodyEquals);
    if (testCase.maxBytes) assert.ok(body.length <= testCase.maxBytes, `${body.length} bytes`);
    if (testCase.headerEcho) {
      const parsed = JSON.parse(body);
      assert.ok(
        parsed.headers.some(
          ([key, value]) => key === testCase.headerEcho[0] && value === testCase.headerEcho[1],
        ),
        JSON.stringify(parsed),
      );
    }
    if (testCase.responseHeaderContains) {
      const [key, fragment] = testCase.responseHeaderContains;
      assert.ok(
        (response.headers.get(key) ?? "").includes(fragment),
        `${key}: ${response.headers.get(key)}`,
      );
    }
    assert.equal(response.headers.get("access-control-allow-origin"), "*");
    assert.equal(response.headers.get("access-control-expose-headers"), "*");
  });
}

test("worker: gRPC is refused with an explanation, never half-implemented", async () => {
  const response = await worker.fetch(new Request(`${BASE}/mock.v1.Mock/Say`, { method: "POST" }));
  assert.equal(response.status, 501);
  const body = await response.json();
  assert.match(body.how, /make mock-api/);
});

test("worker: a preflight is answered without touching a route", async () => {
  const response = await worker.fetch(
    new Request(`${BASE}/post`, {
      method: "OPTIONS",
      headers: { origin: "https://mandalo.dev", "access-control-request-method": "POST" },
    }),
  );
  assert.equal(response.status, 204);
  assert.match(response.headers.get("access-control-allow-methods"), /PATCH/);
});
