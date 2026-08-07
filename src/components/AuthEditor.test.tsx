import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { newDraft, type AuthDraft } from "../lib/draft";
import { AuthEditor } from "./AuthEditor";

function authWith(patch: Partial<AuthDraft>): AuthDraft {
  return { ...newDraft().auth, ...patch };
}

function show(patch: Partial<AuthDraft>, onChange = vi.fn()) {
  return {
    onChange,
    ...render(<AuthEditor auth={authWith(patch)} onChange={onChange} />),
  };
}

describe("AuthEditor inheritance", () => {
  afterEach(cleanup);

  it("says nothing about inheritance when the request owns its auth", () => {
    const { container } = show({ type: "bearer", token: "t", inherited: false });

    expect(container.querySelector(".auth-inherited")).toBeNull();
    expect((screen.getByLabelText("Token") as HTMLInputElement).readOnly).toBe(
      false,
    );
  });

  it("marks an inherited auth and locks the values it does not own", () => {
    const { container } = show({ type: "bearer", token: "t", inherited: true });

    expect(container.querySelector(".auth-inherited")?.textContent).toContain(
      "Inherited from the collection",
    );
    expect((screen.getByLabelText("Token") as HTMLInputElement).readOnly).toBe(
      true,
    );
    expect((screen.getByLabelText("Type") as HTMLSelectElement).disabled).toBe(
      true,
    );
  });

  it("stops inheriting on demand, keeping the values as a starting point", () => {
    const { onChange } = show({ type: "bearer", token: "t", inherited: true });

    fireEvent.click(screen.getByRole("button", { name: "Use its own" }));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ inherited: false, type: "bearer", token: "t" }),
    );
  });
});
