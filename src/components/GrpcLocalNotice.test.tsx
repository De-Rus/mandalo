import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { newDraft, type RequestDraft } from "../lib/draft";
import { GrpcLocalNotice } from "./GrpcLocalNotice";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const global = window as unknown as Record<string, unknown>;

function grpcDraft(url: string): RequestDraft {
  return { ...newDraft("gRPC Say", "grpc"), url };
}

const HOSTED = { grpcUrl: "http://localhost:50051" };

describe("local gRPC notice", () => {
  afterEach(() => {
    delete global.__TAURI_INTERNALS__;
    cleanup();
  });

  it("explains the hosted mock has no gRPC when the target is this machine", () => {
    render(<GrpcLocalNotice draft={grpcDraft("{{grpcUrl}}")} vars={HOSTED} />);

    const note = screen.getByRole("note");
    expect(note.textContent).toContain("cannot serve gRPC");
    expect(note.textContent).toContain("make mock-api");
    expect(note.textContent).toContain("desktop app");
  });

  it("says nothing in the desktop app, which speaks gRPC itself", () => {
    global.__TAURI_INTERNALS__ = {};
    render(<GrpcLocalNotice draft={grpcDraft("{{grpcUrl}}")} vars={HOSTED} />);

    expect(screen.queryByRole("note")).toBeNull();
  });

  it("says nothing when the request points at a real host", () => {
    render(
      <GrpcLocalNotice
        draft={grpcDraft("{{grpcUrl}}")}
        vars={{ grpcUrl: "https://grpc.example.com" }}
      />,
    );

    expect(screen.queryByRole("note")).toBeNull();
  });

  it("says nothing for an HTTP request to the same local port", () => {
    render(
      <GrpcLocalNotice
        draft={{ ...newDraft("Local", "http"), url: "http://localhost:8787" }}
        vars={{}}
      />,
    );

    expect(screen.queryByRole("note")).toBeNull();
  });
});
