import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NewMenu } from "./NewMenu";

afterEach(cleanup);

describe("NewMenu", () => {
  it("offers every creatable type behind one button", () => {
    render(
      <NewMenu
        onNewRequest={() => {}}
        onNewFolder={() => {}}
        onNewCollection={() => {}}
      />,
    );

    expect(screen.queryByText("HTTP Request")).toBeNull();
    fireEvent.click(screen.getByLabelText("New"));

    expect(screen.getByText("HTTP Request")).toBeTruthy();
    expect(screen.getByText("GraphQL Request")).toBeTruthy();
    expect(screen.getByText("gRPC Request")).toBeTruthy();
    expect(screen.getByText("Folder")).toBeTruthy();
    expect(screen.getByText("Collection")).toBeTruthy();
    expect(screen.queryByText("Import…")).toBeNull();
    expect(screen.queryByText("Export…")).toBeNull();
    expect(screen.queryByText("Export bundle…")).toBeNull();
  });

  it("reports the chosen request kind and closes", () => {
    const onNewRequest = vi.fn();
    render(
      <NewMenu
        onNewRequest={onNewRequest}
        onNewFolder={() => {}}
        onNewCollection={() => {}}
      />,
    );

    fireEvent.click(screen.getByLabelText("New"));
    fireEvent.click(screen.getByText("gRPC Request"));

    expect(onNewRequest).toHaveBeenCalledWith("grpc");
    expect(screen.queryByText("gRPC Request")).toBeNull();
  });

  it("names the collection a first request will land in", () => {
    render(
      <NewMenu
        scratch
        onNewRequest={() => {}}
        onNewFolder={null}
        onNewCollection={() => {}}
      />,
    );

    fireEvent.click(screen.getByLabelText("New"));
    expect(screen.getByText("Create in “Scratch”")).toBeTruthy();
  });

  it("hides Folder when there is no collection to put it in", () => {
    render(
      <NewMenu
        onNewRequest={() => {}}
        onNewFolder={null}
        onNewCollection={() => {}}
      />,
    );

    fireEvent.click(screen.getByLabelText("New"));
    expect(screen.queryByText("Folder")).toBeNull();
    expect(screen.getByText("Collection")).toBeTruthy();
  });
});
