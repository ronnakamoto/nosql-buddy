/**
 * Utility functions for formatting and displaying keyboard shortcuts
 */

export const isMac = navigator.platform.startsWith("Mac") || navigator.platform === "iPhone";

export interface ShortcutKeys {
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
  meta?: boolean;
  key: string;
}

/**
 * Format shortcut keys for display based on platform
 */
export function formatShortcut(keys: ShortcutKeys[]): string {
  return keys.map(formatSingleShortcut).join(" or ");
}

/**
 * Format a single shortcut combination for display
 */
export function formatSingleShortcut(shortcut: ShortcutKeys): string {
  const parts: string[] = [];
  
  if (shortcut.ctrl) parts.push(isMac ? "⌃" : "Ctrl");
  if (shortcut.alt) parts.push(isMac ? "⌥" : "Alt");
  if (shortcut.shift) parts.push(isMac ? "⇧" : "Shift");
  if (shortcut.meta) parts.push(isMac ? "⌘" : "Win");
  
  // Handle special keys
  let key = shortcut.key;
  switch (key.toLowerCase()) {
    case "escape":
      key = "Esc";
      break;
    case "enter":
      key = "↵";
      break;
    case "space":
      key = "Space";
      break;
    case "arrowup":
      key = "↑";
      break;
    case "arrowdown":
      key = "↓";
      break;
    case "arrowleft":
      key = "←";
      break;
    case "arrowright":
      key = "→";
      break;
    case "tab":
      key = "Tab";
      break;
    case "backspace":
      key = "⌫";
      break;
    case "delete":
      key = "⌦";
      break;
  }
  
  parts.push(key);
  return parts.join(isMac ? "" : "+");
}

/**
 * Parse a shortcut string like "CmdOrCtrl+K" into ShortcutKeys
 */
export function parseShortcut(shortcutStr: string): ShortcutKeys[] {
  return shortcutStr.split(" or ").map(parseSingleShortcut);
}

/**
 * Parse a single shortcut string
 */
export function parseSingleShortcut(shortcutStr: string): ShortcutKeys {
  const keys = shortcutStr.toLowerCase().split("+");
  const result: ShortcutKeys = {
    ctrl: false,
    alt: false,
    shift: false,
    meta: false,
    key: ""
  };
  
  keys.forEach(key => {
    switch (key) {
      case "ctrl":
      case "cmdorctrl":
        result.ctrl = true;
        break;
      case "meta":
      case "cmd":
      case "⌘":
        result.meta = true;
        break;
      case "alt":
      case "⌥":
        result.alt = true;
        break;
      case "shift":
      case "⇧":
        result.shift = true;
        break;
      default:
        result.key = key.toUpperCase();
        break;
    }
  });
  
  return result;
}

/**
 * Check if an event matches a shortcut
 */
export function eventMatchesShortcut(event: KeyboardEvent, shortcut: ShortcutKeys): boolean {
  return (
    // Modifier flags are optional on `ShortcutKeys`; an omitted flag means
    // "must not be pressed". Coerce undefined → false so `false === undefined`
    // doesn't make every `COMMON_SHORTCUTS` entry (which omits unused
    // modifiers) fail to match.
    event.ctrlKey === (shortcut.ctrl ?? false) &&
    event.altKey === (shortcut.alt ?? false) &&
    event.shiftKey === (shortcut.shift ?? false) &&
    event.metaKey === (shortcut.meta ?? false) &&
    event.key.toLowerCase() === shortcut.key.toLowerCase()
  );
}

/**
 * Common shortcut combinations
 */
export const COMMON_SHORTCUTS = {
  SAVE: [{ ctrl: true, key: "s" }],
  COPY: [{ ctrl: true, key: "c" }],
  PASTE: [{ ctrl: true, key: "v" }],
  CUT: [{ ctrl: true, key: "x" }],
  UNDO: [{ ctrl: true, key: "z" }],
  REDO: [{ ctrl: true, shift: true, key: "z" }],
  SELECT_ALL: [{ ctrl: true, key: "a" }],
  FIND: [{ ctrl: true, key: "f" }],
  COMMAND_PALETTE: [{ meta: true, key: "k" }],
  NEW_TAB: [{ meta: true, key: "t" }],
  NEW_CONNECTION: [{ meta: true, key: "n" }],
  SHORTCUTS_HELP: [{ meta: true, key: "?" }],
  TOGGLE_SIDEBAR: [{ meta: true, key: "b" }],
  ESCAPE: [{ key: "escape" }],
  ENTER: [{ key: "enter" }],
  TAB: [{ key: "tab" }],
  ARROW_UP: [{ key: "arrowup" }],
  ARROW_DOWN: [{ key: "arrowdown" }],
} as const;