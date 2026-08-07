import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EnvironmentView } from "../lib/api";
import { newDraft, type RequestDraft } from "../lib/draft";
import { useEnv } from "../store/env";
import { UrlBar } from "./UrlBar";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const env: EnvironmentView = {
  name: "hosted",
  vars: {
    baseUrl: {
      shared: true,
      secret: false,
      value: "https://api.mandalo.dev",
      set: true,
      source: "file",
    },
    apiKey: {
      shared: false,
      secret: true,
      value: null,
      hosts: [],
      set: true,
      source: "local",
    },
    unsetKey: {
      shared: false,
      secret: true,
      value: null,
      hosts: [],
      set: false,
    },
  },
};

function draftWith(patch: Partial<RequestDraft>): RequestDraft {
  return { ...newDraft("R", "http"), ...patch };
}

function show(draft: RequestDraft) {
  const vars = { baseUrl: "https://api.mandalo.dev" };
  return render(
    <UrlBar
      draft={draft}
      vars={vars}
      sending={false}
      dirty={false}
      streamPhase={null}
      onPatch={() => {}}
      onSend={() => {}}
      onSave={() => {}}
    />,
  );
}

let push: ((patch: Partial<RequestDraft>) => void) | null = null;

function Harness({ initial }: { initial: RequestDraft }) {
  const [draft, setDraft] = useState(initial);
  push = (patch) => setDraft((d) => ({ ...d, ...patch }));
  return (
    <UrlBar
      draft={draft}
      vars={{ baseUrl: "https://api.mandalo.dev" }}
      sending={false}
      dirty={false}
      streamPhase={null}
      onPatch={(patch) => setDraft((d) => ({ ...d, ...patch }))}
      onSend={() => {}}
      onSave={() => {}}
    />
  );
}

function live(patch: Partial<RequestDraft>) {
  const view = render(<Harness initial={draftWith(patch)} />);
  return { ...view, input: screen.getByLabelText("URL") as HTMLInputElement };
}

describe("UrlBar query string", () => {
  beforeEach(() => {
    useEnv.setState({ envs: [env], selected: "hosted" });
  });

  afterEach(cleanup);

  it("shows the params table in the URL, not just the bare path", () => {
    const { input } = live({
      url: "{{baseUrl}}/users",
      params: [
        { id: "1", key: "page", value: "1", enabled: true },
        { id: "2", key: "q", value: "{{baseUrl}} me", enabled: true },
        { id: "3", key: "off", value: "no", enabled: false },
      ],
    });

    expect(input.value).toBe("{{baseUrl}}/users?page=1&q={{baseUrl}}%20me");
  });

  it("populates the params table from a query string typed into the URL", () => {
    const patch = vi.fn();
    render(
      <UrlBar
        draft={draftWith({ url: "https://x.dev/users" })}
        vars={{}}
        sending={false}
        dirty={false}
        streamPhase={null}
        onPatch={patch}
        onSend={() => {}}
        onSave={() => {}}
      />,
    );

    fireEvent.change(screen.getByLabelText("URL"), {
      target: { value: "https://x.dev/users?page=2&q={{term}}%20b" },
    });

    expect(patch).toHaveBeenCalledTimes(1);
    const next = patch.mock.calls[0][0] as Partial<RequestDraft>;
    expect(next.url).toBe("https://x.dev/users");
    expect(next.params?.map((r) => [r.key, r.value])).toEqual([
      ["page", "2"],
      ["q", "{{term}} b"],
      ["", ""],
    ]);
  });

  it("keeps a half-typed pair on screen instead of rewriting it under the caret", () => {
    const { input } = live({ url: "https://x.dev/users" });

    fireEvent.change(input, { target: { value: "https://x.dev/users?" } });
    expect(input.value).toBe("https://x.dev/users?");

    fireEvent.change(input, { target: { value: "https://x.dev/users?pa" } });
    expect(input.value).toBe("https://x.dev/users?pa");
  });

  it("gives up the half-typed text as soon as the params table moves", () => {
    const { input } = live({ url: "https://x.dev/users" });

    fireEvent.change(input, { target: { value: "https://x.dev/users?a" } });
    expect(input.value).toBe("https://x.dev/users?a");

    act(() => {
      push?.({ params: [{ id: "9", key: "a", value: "1", enabled: true }] });
    });

    expect(input.value).toBe("https://x.dev/users?a=1");
  });

  it("draws a variable inside a param value as a token", () => {
    const { container } = live({
      url: "https://x.dev/users",
      params: [{ id: "1", key: "q", value: "{{baseUrl}}", enabled: true }],
    });

    expect(container.querySelector(".var-token")?.textContent).toBe(
      "{{baseUrl}}",
    );
  });
});

describe("UrlBar variables", () => {
  beforeEach(() => {
    useEnv.setState({ envs: [env], selected: "hosted" });
  });

  afterEach(cleanup);

  it("shows no variable strip when everything resolves", () => {
    const { container } = show(
      draftWith({
        url: "{{baseUrl}}/users",
        headers: [
          { id: "1", key: "Authorization", value: "{{apiKey}}", enabled: true },
        ],
      }),
    );

    expect(container.querySelector(".var-strip")).toBeNull();
    expect(container.querySelector(".var-pill")).toBeNull();
    expect(container.querySelector(".var-warning")).toBeNull();
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("still tells you about a URL variable on hover", () => {
    const { container } = show(draftWith({ url: "{{baseUrl}}/users" }));

    fireEvent.mouseEnter(container.querySelector(".var-token") as HTMLElement);
    expect(screen.getByRole("tooltip").textContent).toContain(
      "https://api.mandalo.dev",
    );
  });

  it("warns about an undefined variable the URL never draws", () => {
    const { container } = show(
      draftWith({
        url: "{{baseUrl}}/users",
        headers: [
          { id: "1", key: "X-Tenant", value: "{{tenant}}", enabled: true },
        ],
      }),
    );

    const warning = container.querySelector(".var-warning");
    expect(warning?.textContent).toContain("{{tenant}}");
    expect(warning?.textContent).toContain("not defined in environment hosted");
    expect(warning?.textContent).toContain("The request will fail until it is set.");
  });

  it("counts a secret nothing holds a value for as unresolved", () => {
    const { container } = show(
      draftWith({
        url: "{{baseUrl}}/users",
        headers: [
          { id: "1", key: "Authorization", value: "{{unsetKey}}", enabled: true },
        ],
      }),
    );

    const warning = container.querySelector(".var-warning");
    expect(warning?.textContent).toContain("{{unsetKey}}");
    expect(warning?.textContent).toContain("nothing on this machine holds");
  });

  it("says both kinds when a run has an undefined name and an unset secret", () => {
    const { container } = show(
      draftWith({
        url: "{{tenant}}/users",
        headers: [
          { id: "1", key: "Authorization", value: "{{unsetKey}}", enabled: true },
        ],
      }),
    );

    expect(container.querySelector(".var-warning")?.textContent).toContain(
      "not defined, or not set on this machine",
    );
  });

  it("ignores a variable in a disabled row", () => {
    const { container } = show(
      draftWith({
        url: "{{baseUrl}}/users",
        headers: [
          { id: "1", key: "X-Tenant", value: "{{tenant}}", enabled: false },
        ],
      }),
    );

    expect(container.querySelector(".var-warning")).toBeNull();
  });

  it("names every unresolved variable, and each one is hoverable", () => {
    const { container } = show(
      draftWith({
        url: "{{missingA}}/users",
        headers: [
          { id: "1", key: "X-B", value: "{{missingB}}", enabled: true },
        ],
      }),
    );

    const warning = container.querySelector(".var-warning") as HTMLElement;
    expect(warning.textContent).toContain("{{missingA}}");
    expect(warning.textContent).toContain("{{missingB}}");
    expect(warning.textContent).toContain("they are set");

    const tokens = warning.querySelectorAll(".var-token");
    expect(tokens).toHaveLength(2);
    fireEvent.focus(tokens[1] as HTMLElement);
    expect(screen.getByRole("tooltip").textContent).toContain("{{missingB}}");
  });

  it("says so when nothing is selected to define variables in", () => {
    useEnv.setState({ envs: [], selected: null });
    const { container } = show(draftWith({ url: "{{tenant}}/users" }));

    expect(container.querySelector(".var-warning")?.textContent).toContain(
      "no environment is selected",
    );
  });

  it("keeps the URL text itself out of the accessibility tree twice over", () => {
    const { container } = show(draftWith({ url: "{{baseUrl}}/users" }));

    const plain = Array.from(
      container.querySelectorAll(".url-overlay-inner > span"),
    ).filter((el) => !el.classList.contains("var-token"));
    expect(plain.length).toBeGreaterThan(0);
    for (const el of plain)
      expect(el.getAttribute("aria-hidden")).toBe("true");
    expect(
      container.querySelector(".var-token")?.getAttribute("aria-hidden"),
    ).toBeNull();
  });
});
