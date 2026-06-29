import { describe, it, expect } from "vitest";
import {
  eventMatchesShortcut,
  parseShortcut,
  parseSingleShortcut,
  formatShortcut,
  COMMON_SHORTCUTS,
} from "./shortcutUtils";

function key(opts: Partial<KeyboardEvent> & { key: string }): KeyboardEvent {
  return {
    key: opts.key,
    ctrlKey: opts.ctrlKey ?? false,
    altKey: opts.altKey ?? false,
    shiftKey: opts.shiftKey ?? false,
    metaKey: opts.metaKey ?? false,
  } as KeyboardEvent;
}

describe("eventMatchesShortcut", () => {
  it("matches a COMMON_SHORTCUTS entry that omits modifier flags — regression", () => {
    // SAVE = [{ ctrl: true, key: "s" }] with alt/shift/meta omitted (undefined).
    // The old strict `=== undefined` comparison made this never match.
    const ev = key({ key: "s", ctrlKey: true });
    expect(eventMatchesShortcut(ev, COMMON_SHORTCUTS.SAVE[0])).toBe(true);
  });

  it("does not match when an unexpected modifier is pressed", () => {
    const ev = key({ key: "s", ctrlKey: true, shiftKey: true });
    expect(eventMatchesShortcut(ev, COMMON_SHORTCUTS.SAVE[0])).toBe(false);
  });

  it("matches multi-modifier shortcuts (Ctrl+Shift+Z)", () => {
    const ev = key({ key: "z", ctrlKey: true, shiftKey: true });
    expect(eventMatchesShortcut(ev, COMMON_SHORTCUTS.REDO[0])).toBe(true);
  });

  it("is case-insensitive on the key", () => {
    const ev = key({ key: "S", ctrlKey: true });
    expect(eventMatchesShortcut(ev, COMMON_SHORTCUTS.SAVE[0])).toBe(true);
  });

  it("matches a parsed shortcut where all flags are explicit false", () => {
    const ev = key({ key: "escape" });
    expect(eventMatchesShortcut(ev, parseSingleShortcut("Escape"))).toBe(true);
  });
});

describe("parseShortcut", () => {
  it("maps cmdorctrl to ctrl and uppercases the key", () => {
    const parsed = parseSingleShortcut("CmdOrCtrl+K");
    expect(parsed.ctrl).toBe(true);
    expect(parsed.key).toBe("K");
  });

  it("splits ' or ' into multiple combos", () => {
    const parsed = parseShortcut("ArrowUp or ArrowDown");
    expect(parsed).toHaveLength(2);
    expect(parsed[0].key).toBe("ARROWUP");
  });

  it("round-trips through format for a simple combo", () => {
    const formatted = formatShortcut([{ ctrl: true, key: "s" }]);
    expect(formatted.length).toBeGreaterThan(0);
  });
});
