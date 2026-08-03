import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ResponseData } from "../lib/api";
import { ResponsePane } from "./ResponsePane";

function data(patch: Partial<ResponseData> = {}): ResponseData {
  return {
    status: 200,
    statusText: "OK",
    headers: [],
    body: '{"a":1}',
    binary: false,
    durationMs: 12,
    sizeBytes: 7,
    ...patch,
  };
}

describe("ResponsePane", () => {
  afterEach(cleanup);

  it("pretty prints a textual JSON body and shows no binary chip", () => {
    const { container } = render(
      <ResponsePane response={{ phase: "http", data: data() }} />,
    );

    expect(container.querySelector(".response-body")?.textContent).toBe(
      '{\n  "a": 1\n}',
    );
    expect(screen.queryByText("binary")).toBeNull();
    expect(screen.queryByText(/not valid UTF-8/)).toBeNull();
  });

  it("marks a binary body with a chip, a note, and leaves it unformatted", () => {
    const { container } = render(
      <ResponsePane
        response={{ phase: "http", data: data({ binary: true, sizeBytes: 5120 }) }}
      />,
    );

    expect(screen.getByText("binary")).toBeTruthy();
    expect(screen.getByText(/not valid UTF-8/)).toBeTruthy();
    expect(container.querySelector(".response-body")?.textContent).toBe(
      '{"a":1}',
    );
    expect(screen.getByText("5.0 KB")).toBeTruthy();
  });
});
