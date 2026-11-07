import { describe, expect, it } from "vitest";
import type { AuthSettings, Doc, SlicerState } from "./types";
import {
  authConfigured,
  buildAuthInput,
  elementTypeLabel,
  esc,
  fileName,
  filterDocs,
  fmtDate,
  plural,
  refreshSummary,
  secretPlaceholder,
  selectedSlicer,
  slicerName,
  slicerOptionLabel,
  type AuthForm,
} from "./ui";

const doc = (name: string): Doc => ({
  id: name,
  name,
  modifiedAt: null,
  workspaceId: null,
  thumbHref: null,
});

const auth = (over: Partial<AuthSettings> = {}): AuthSettings => ({
  mode: "apiKey",
  hasApiKeys: false,
  username: "",
  hasPassword: false,
  hasSession: false,
  ...over,
});

const form = (over: Partial<AuthForm> = {}): AuthForm => ({
  mode: "apiKey",
  accessKey: "",
  secretKey: "",
  username: "",
  password: "",
  session: "",
  ...over,
});

describe("formatting", () => {
  it("escapes HTML special characters", () => {
    expect(esc('<a href="x">&</a>')).toBe("&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;");
    expect(esc(null)).toBe("");
  });

  it("formats dates and tolerates missing ones", () => {
    expect(fmtDate(null)).toBe("");
    expect(fmtDate("2026-03-04T12:00:00Z")).toMatch(/2026/);
  });

  it("pluralizes", () => {
    expect(plural(1, "doc")).toBe("1 doc");
    expect(plural(0, "doc")).toBe("0 docs");
    expect(plural(3, "API call")).toBe("3 API calls");
  });

  it("summarizes a refresh", () => {
    expect(refreshSummary(0, 1)).toBe("No changes \u00b7 1 API call");
    expect(refreshSummary(undefined, 0)).toBe("No changes \u00b7 0 API calls");
    expect(refreshSummary(2, 3)).toBe("2 new/changed docs \u00b7 3 API calls");
  });

  it("takes the file name of Windows and POSIX paths", () => {
    expect(fileName("C:\\Users\\me\\exports\\a\\b.stl")).toBe("b.stl");
    expect(fileName("/home/me/b.stl")).toBe("b.stl");
    expect(fileName("b.stl")).toBe("b.stl");
  });
});

describe("filterDocs", () => {
  const docs = [doc("Bracket"), doc("button holders"), doc("Lid")];

  it("matches case-insensitively on a trimmed query", () => {
    expect(filterDocs(docs, "  BUT ").map((d) => d.name)).toEqual(["button holders"]);
  });

  it("returns everything for an empty query", () => {
    expect(filterDocs(docs, "  ")).toHaveLength(3);
  });
});

describe("elementTypeLabel", () => {
  it("labels part studios, blobs by extension, and other types", () => {
    expect(elementTypeLabel({ elementType: "PARTSTUDIO", name: "Part Studio 1" })).toBe(
      "part studio",
    );
    expect(elementTypeLabel({ elementType: "BLOB", name: "Bracket.stl" })).toBe("stl");
    expect(elementTypeLabel({ elementType: "BLOB", name: "README" })).toBe("file");
    expect(elementTypeLabel({ elementType: "ASSEMBLY", name: "Asm" })).toBe("assembly");
  });
});

describe("authConfigured", () => {
  it("checks what the selected mode needs", () => {
    expect(authConfigured(auth({ hasApiKeys: true }))).toBe(true);
    expect(authConfigured(auth())).toBe(false);
    expect(authConfigured(auth({ mode: "password", hasPassword: true }))).toBe(false);
    expect(authConfigured(auth({ mode: "password", username: "a@b.c", hasPassword: true }))).toBe(
      true,
    );
    expect(authConfigured(auth({ mode: "session", hasApiKeys: true }))).toBe(false);
    expect(authConfigured(auth({ mode: "session", hasSession: true }))).toBe(true);
  });
});

describe("buildAuthInput", () => {
  it("omits blank secrets so the stored ones are kept", () => {
    expect(buildAuthInput(form(), auth({ hasApiKeys: true }))).toEqual({ mode: "apiKey" });
  });

  it("sends filled API keys trimmed, and only the current mode's fields", () => {
    expect(
      buildAuthInput(form({ accessKey: " on_a ", secretKey: "s", password: "p" }), null),
    ).toEqual({ mode: "apiKey", accessKey: "on_a", secretKey: "s" });
  });

  it("sends the username only when it changed", () => {
    const current = auth({ mode: "password", username: "a@b.c", hasPassword: true });
    expect(buildAuthInput(form({ mode: "password", username: "a@b.c" }), current)).toEqual({
      mode: "password",
    });
    expect(
      buildAuthInput(form({ mode: "password", username: "x@y.z", password: "pw" }), current),
    ).toEqual({ mode: "password", username: "x@y.z", password: "pw" });
  });

  it("sends a pasted session", () => {
    expect(buildAuthInput(form({ mode: "session", session: " c=1 " }), null)).toEqual({
      mode: "session",
      session: "c=1",
    });
  });

  it("marks stored secrets in the placeholder", () => {
    expect(secretPlaceholder(true, "secret")).toBe("stored - leave blank to keep");
    expect(secretPlaceholder(false, "secret")).toBe("secret");
  });
});

describe("slicer helpers", () => {
  const bambu = { name: "Bambu Studio", path: "C:\\bambu.exe", manual: false };
  const mine = { name: "my-slicer", path: "D:\\my-slicer.exe", manual: true };

  it("finds the selected slicer and its name", () => {
    const state: SlicerState = { slicers: [bambu, mine], selected: mine.path };
    expect(selectedSlicer(state)).toBe(mine);
    expect(slicerName(state)).toBe("my-slicer");
  });

  it("falls back when nothing is selected", () => {
    expect(selectedSlicer(null)).toBeNull();
    expect(selectedSlicer({ slicers: [bambu], selected: null })).toBeNull();
    expect(selectedSlicer({ slicers: [], selected: "gone.exe" })).toBeNull();
    expect(slicerName(null)).toBe("your slicer");
  });

  it("marks manual entries in the dropdown", () => {
    expect(slicerOptionLabel(bambu)).toBe("Bambu Studio");
    expect(slicerOptionLabel(mine)).toBe("my-slicer (added manually)");
  });
});
