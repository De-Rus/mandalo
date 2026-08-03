export interface Snippet {
  label: string;
  code: string;
}

export const PRE_SNIPPETS: Snippet[] = [
  {
    label: "Get an environment variable",
    code: 'const token = pm.environment.get("token");\n',
  },
  {
    label: "Set an environment variable",
    code: 'pm.environment.set("token", "value");\n',
  },
  {
    label: "Add a request header",
    code: 'pm.request.headers.add("X-Trace-Id", crypto.randomUUID());\n',
  },
  {
    label: "Timestamp variable",
    code: 'pm.environment.set("now", String(Date.now()));\n',
  },
  {
    label: "Log a value",
    code: "console.log(pm.request.url);\n",
  },
];

export const TEST_SNIPPETS: Snippet[] = [
  {
    label: "Status code: Code is 200",
    code:
      'pm.test("Status code is 200", function () {\n' +
      "  pm.response.to.have.status(200);\n" +
      "});\n",
  },
  {
    label: "Response body: JSON value check",
    code:
      'pm.test("Body has the expected id", function () {\n' +
      "  const json = pm.response.json();\n" +
      "  pm.expect(json.id).to.eql(1);\n" +
      "});\n",
  },
  {
    label: "Response body: contains string",
    code:
      'pm.test("Body contains ok", function () {\n' +
      '  pm.expect(pm.response.text()).to.include("ok");\n' +
      "});\n",
  },
  {
    label: "Response headers: Content-Type header check",
    code:
      'pm.test("Content-Type is present", function () {\n' +
      '  pm.response.to.have.header("Content-Type");\n' +
      "});\n",
  },
  {
    label: "Response time is less than 200ms",
    code:
      'pm.test("Response time is less than 200ms", function () {\n' +
      "  pm.expect(pm.response.responseTime).to.be.below(200);\n" +
      "});\n",
  },
  {
    label: "Set an environment variable",
    code: 'pm.environment.set("userId", pm.response.json().id);\n',
  },
  {
    label: "Get an environment variable",
    code: 'const userId = pm.environment.get("userId");\n',
  },
];

export function insertAt(
  text: string,
  cursor: number,
  snippet: string,
): { text: string; cursor: number } {
  const before = text.slice(0, cursor);
  const after = text.slice(cursor);
  const prefix = before === "" || before.endsWith("\n") ? "" : "\n";
  const inserted = `${prefix}${snippet}`;
  return {
    text: `${before}${inserted}${after}`,
    cursor: cursor + inserted.length,
  };
}
