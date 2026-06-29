import { CompletionContext } from "@codemirror/autocomplete";
import { EditorState } from "@codemirror/state";
import { describe, expect, it, vi } from "vitest";
import { itemToCompletion, makeShellCompletionSource } from "./shellCompletion";

describe("shell completion adapter", () => {
  function context(doc: string, pos = doc.length, explicit = false) {
    return new CompletionContext(EditorState.create({ doc }), pos, explicit);
  }

  it("maps backend items into CodeMirror completions", () => {
    expect(itemToCompletion({ label: "find", detail: "method" })).toMatchObject({
      label: "find",
      detail: "method",
      type: "function",
      apply: "find",
    });
    expect(itemToCompletion({ label: "movies", detail: "collection" })).toMatchObject({
      label: "movies",
      type: "property",
    });
  });

  it("asks the backend with text before the cursor and returns replacement span", async () => {
    const request = vi.fn().mockResolvedValue({
      kind: { kind: "methods", collection: "movies" },
      items: [{ label: "findOne", detail: "method" }],
    });
    const source = makeShellCompletionSource({ request });

    const result = await source(context("db.movies.findO"));

    expect(request).toHaveBeenCalledWith({ textBeforeCursor: "db.movies.findO" });
    expect(result).toMatchObject({
      from: "db.movies.".length,
      to: "db.movies.findO".length,
      filter: false,
      options: [{ label: "findOne", detail: "method", apply: "findOne" }],
    });
  });

  it("opens after dot triggers even without an identifier", async () => {
    const request = vi.fn().mockResolvedValue({
      kind: { kind: "methods", collection: "movies" },
      items: [{ label: "find", detail: "method" }],
    });
    const source = makeShellCompletionSource({ request });

    const result = await source(context("db.movies."));

    expect(request).toHaveBeenCalledOnce();
    expect(result?.from).toBe("db.movies.".length);
    expect(result?.options).toHaveLength(1);
  });

  it("does not call the backend for idle implicit completion", async () => {
    const request = vi.fn();
    const source = makeShellCompletionSource({ request });

    await expect(source(context("db.movies.find({ year: 2010 })"))).resolves.toBeNull();
    expect(request).not.toHaveBeenCalled();
  });

  it("returns null for backend none responses", async () => {
    const request = vi.fn().mockResolvedValue({
      kind: { kind: "none" },
      items: [{ label: "ignored", detail: "ignored" }],
    });
    const source = makeShellCompletionSource({ request });

    await expect(source(context("db."))).resolves.toBeNull();
  });
});
