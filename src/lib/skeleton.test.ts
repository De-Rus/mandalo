import { describe, expect, it } from "vitest";
import type { MessageShape } from "./api";
import { replaceable, skeletonFor, unknownFields } from "./skeleton";

const address: MessageShape = {
  name: "mock.v1.Address",
  fields: [
    { name: "city", type: "string", repeated: false, message: null, enumValues: [] },
  ],
};

const getUser: MessageShape = {
  name: "mock.v1.GetUserRequest",
  fields: [
    { name: "id", type: "string", repeated: false, message: null, enumValues: [] },
    { name: "age", type: "number", repeated: false, message: null, enumValues: [] },
    { name: "active", type: "bool", repeated: false, message: null, enumValues: [] },
    { name: "tags", type: "string", repeated: true, message: null, enumValues: [] },
    {
      name: "tier",
      type: "enum",
      repeated: false,
      message: null,
      enumValues: ["TIER_UNSPECIFIED", "TIER_PRO"],
    },
    {
      name: "address",
      type: "message",
      repeated: false,
      message: address,
      enumValues: [],
    },
  ],
};

const say: MessageShape = {
  name: "mock.v1.EchoRequest",
  fields: [
    { name: "text", type: "string", repeated: false, message: null, enumValues: [] },
    { name: "count", type: "number", repeated: false, message: null, enumValues: [] },
  ],
};

describe("message skeletons", () => {
  it("writes one placeholder per field, nested and repeated included", () => {
    expect(JSON.parse(skeletonFor(getUser))).toEqual({
      id: "",
      age: 0,
      active: false,
      tags: [],
      tier: "TIER_UNSPECIFIED",
      address: { city: "" },
    });
  });

  it("is pretty-printed the way the body editor writes JSON", () => {
    expect(skeletonFor(say)).toBe('{\n  "text": "",\n  "count": 0\n}');
  });
});

describe("skeleton replacement decision", () => {
  it("replaces an empty message", () => {
    expect(replaceable("   ", skeletonFor(say))).toBe(true);
  });

  it("replaces the previous method's untouched skeleton, whitespace aside", () => {
    expect(replaceable('{"text":"","count":0}', skeletonFor(say))).toBe(true);
  });

  it("keeps a message the user wrote", () => {
    expect(replaceable('{"text": "hola", "count": 21}', skeletonFor(say))).toBe(
      false,
    );
  });

  it("keeps anything it cannot parse", () => {
    expect(replaceable('{"text": ', skeletonFor(say))).toBe(false);
  });

  it("keeps a written message when the previous shape is unknown", () => {
    expect(replaceable('{"text": ""}', null)).toBe(false);
  });
});

describe("unknown fields", () => {
  it("names the fields the selected method does not have", () => {
    expect(unknownFields('{"text": "hola", "id": "u-1"}', getUser)).toEqual([
      "text",
    ]);
  });

  it("says nothing about unparsable JSON", () => {
    expect(unknownFields("{", getUser)).toEqual([]);
  });
});
