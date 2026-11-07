import { describe, expect, it } from "vitest";
import { elementsStale, exportRel, isStale3mfBlob, sanitize, thumbKey } from "./api.js";

describe("sanitize", () => {
  it("replaces characters Windows rejects in path segments", () => {
    expect(sanitize('a/b\\c:d*e?f"g<h>i|j')).toBe("a_b_c_d_e_f_g_h_i_j");
  });

  it("collapses whitespace and trims trailing dots and spaces", () => {
    expect(sanitize("  my   part. . ")).toBe("my part");
  });

  it("falls back to 'model' for empty or all-invalid names", () => {
    expect(sanitize("")).toBe("model");
    expect(sanitize(null)).toBe("model");
    expect(sanitize("...")).toBe("model");
  });

  it("caps the length at 80 characters", () => {
    expect(sanitize("x".repeat(200))).toHaveLength(80);
  });
});

describe("isStale3mfBlob", () => {
  it("flags 3MF blobs by data type or file name", () => {
    expect(isStale3mfBlob({ elementType: "BLOB", dataType: "application/3mf" })).toBe(true);
    expect(isStale3mfBlob({ elementType: "BLOB", name: "Plate.3MF" })).toBe(true);
  });

  it("keeps STL blobs and part studios", () => {
    expect(isStale3mfBlob({ elementType: "BLOB", dataType: "model/stl", name: "a.stl" })).toBe(
      false,
    );
    expect(isStale3mfBlob({ elementType: "PARTSTUDIO", name: "x.3mf" })).toBe(false);
  });
});

describe("elementsStale", () => {
  it("is never stale when the doc has no modifiedAt", () => {
    expect(elementsStale({ docModifiedAt: "2026-01-01T00:00:00Z" }, {})).toBe(false);
  });

  it("compares against the recorded doc modifiedAt", () => {
    const old = { docModifiedAt: "2026-01-01T00:00:00Z" };
    expect(elementsStale(old, { modifiedAt: "2026-01-01T00:00:00Z" })).toBe(false);
    expect(elementsStale(old, { modifiedAt: "2026-02-01T00:00:00Z" })).toBe(true);
  });

  it("falls back to fetchedAt for pre-upgrade caches", () => {
    const old = { fetchedAt: "2026-01-15T00:00:00Z" };
    expect(elementsStale(old, { modifiedAt: "2026-01-01T00:00:00Z" })).toBe(false);
    expect(elementsStale(old, { modifiedAt: "2026-02-01T00:00:00Z" })).toBe(true);
  });
});

describe("thumbKey", () => {
  it("stamps the key with the thumbnail's t parameter", () => {
    expect(thumbKey({ id: "d1", thumbHref: "https://cad.onshape.com/thumb?t=12:34-ab" })).toBe(
      "d1-1234ab",
    );
  });

  it("uses -0 when there is no t parameter or the href is invalid", () => {
    expect(thumbKey({ id: "d1", thumbHref: "https://cad.onshape.com/thumb" })).toBe("d1-0");
    expect(thumbKey({ id: "d1", thumbHref: "not a url" })).toBe("d1-0");
  });
});

describe("exportRel", () => {
  const doc = { name: "My Doc" };
  const el = { name: "Bracket.stl" };

  it("names a whole-tab export after the tab, without a doubled extension", () => {
    expect(exportRel(doc, el, null, ".stl")).toEqual({
      dir: "exports/My Doc",
      rel: "exports/My Doc/Bracket.stl",
    });
  });

  it("nests a single-part export under the tab", () => {
    expect(exportRel(doc, el, { name: "Arm: left" }, ".stl")).toEqual({
      dir: "exports/My Doc/Bracket.stl",
      rel: "exports/My Doc/Bracket.stl/Arm_ left.stl",
    });
  });
});
