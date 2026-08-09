import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { newDraft, type RequestDraft } from "../lib/draft";
import { Workbench } from "./Workbench";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

function mount(patch: Partial<RequestDraft>, onPatch = vi.fn()) {
  const draft = { ...newDraft("R", "http"), ...patch };
  return render(
    <Workbench
      draft={draft}
      vars={{}}
      sending={false}
      dirty={false}
      streamPhase={null}
      onPatch={onPatch}
      onSend={() => {}}
      onSave={() => {}}
    />,
  );
}

function show(patch: Partial<RequestDraft>, onPatch = vi.fn()) {
  const view = mount(patch, onPatch);
  fireEvent.click(screen.getByRole("tab", { name: /Settings/ }));
  return view;
}

function description(): HTMLTextAreaElement {
  return screen.getByLabelText("Description") as HTMLTextAreaElement;
}

describe("Workbench description", () => {
  afterEach(cleanup);

  it("cannot be edited once the request lives in a text file", () => {
    const onPatch = vi.fn();
    show({ path: "users.http#0" }, onPatch);

    expect(description().readOnly).toBe(true);
    fireEvent.change(description(), { target: { value: "x" } });
    expect(onPatch).not.toHaveBeenCalled();
  });

  it("says where a description lives instead, naming the file", () => {
    const { container } = show({ path: "auth/login.http#2" });

    const hints = Array.from(container.querySelectorAll(".settings-hint"))
      .map((el) => el.textContent)
      .join(" ");
    expect(hints).toContain("comment lines above the request");
    expect(hints).toContain("auth/login.http");
    expect(hints).not.toContain("#2");
  });

  it("stays editable before a file exists, where the format can render it", () => {
    const onPatch = vi.fn();
    show({ path: null }, onPatch);

    expect(description().readOnly).toBe(false);
    fireEvent.change(description(), { target: { value: "why this exists" } });
    expect(onPatch).toHaveBeenCalledWith({ description: "why this exists" });
  });
});
