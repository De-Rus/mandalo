import { createServer } from "node:net";
import { describe, expect, it } from "vitest";
import { NetworkError, send } from "../../src/engine/transport";
import type { PreparedRequest } from "../../src/engine/prepare";

// A port the OS just handed out and released refuses immediately, without
// tripping the fetch spec's bad-port list the way a hardcoded low port would.
function refusedPort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.listen(0, "127.0.0.1", () => {
      const port = (server.address() as { port: number }).port;
      server.close((error) => (error ? reject(error) : resolve(port)));
    });
  });
}

describe("transport errors", () => {
  it("names the refused connection instead of a bare 'fetch failed'", async () => {
    const DEAD: PreparedRequest = {
      method: "GET",
      url: `http://127.0.0.1:${await refusedPort()}/health`,
      headers: [],
      body: null,
    } as unknown as PreparedRequest;
    const error = await send(DEAD).then(
      () => {
        throw new Error("expected the send to fail");
      },
      (raised: unknown) => raised,
    );
    expect(error).toBeInstanceOf(NetworkError);
    expect((error as NetworkError).message).toMatch(/ECONNREFUSED|ENETUNREACH|EACCES/);
    expect((error as NetworkError).message).not.toMatch(/^fetch failed:?\s*$/);
  });
});
