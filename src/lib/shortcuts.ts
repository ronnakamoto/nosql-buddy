/**
 * Centralized keyboard shortcuts registry for NoSQLBuddy
 * This file defines all available keyboard shortcuts and their descriptions
 */

import { formatShortcut, parseShortcut } from "./shortcutUtils";

export interface Shortcut {
  id: string;
  keys: string[];
  description: string;
  category: ShortcutCategory;
  context?: string;
}

export type ShortcutCategory = 
  | "global"
  | "navigation"
  | "editing"
  | "modal"
  | "query"
  | "shell"
  | "data-model";

export const shortcuts: Shortcut[] = [
  // Global shortcuts
  {
    id: "command-palette",
    keys: ["CmdOrCtrl+K"],
    description: "Open command palette",
    category: "global",
  },
  {
    id: "new-connection",
    keys: ["CmdOrCtrl+N"],
    description: "New connection",
    category: "global",
  },
  {
    id: "new-tab",
    keys: ["CmdOrCtrl+T"],
    description: "New query tab",
    category: "global",
  },
  {
    id: "toggle-sidebar",
    keys: ["CmdOrCtrl+B"],
    description: "Toggle sidebar visibility",
    category: "global",
  },
  {
    id: "shortcuts-map",
    keys: ["CmdOrCtrl+?"],
    description: "Show keyboard shortcuts",
    category: "global",
  },

  // Navigation shortcuts
  {
    id: "escape-modal",
    keys: ["Escape"],
    description: "Close modal or dropdown",
    category: "navigation",
  },
  {
    id: "tab-navigation",
    keys: ["Enter"],
    description: "Activate selected tab or item",
    category: "navigation",
  },
  {
    id: "arrow-navigation",
    keys: ["ArrowUp", "ArrowDown"],
    description: "Navigate up/down in lists",
    category: "navigation",
  },

  // Editing shortcuts
  {
    id: "autocomplete-accept",
    keys: ["Enter", "Tab"],
    description: "Accept autocomplete suggestion",
    category: "editing",
  },
  {
    id: "autocomplete-dismiss",
    keys: ["Escape"],
    description: "Dismiss autocomplete",
    category: "editing",
  },
  {
    id: "cell-edit-save",
    keys: ["CmdOrCtrl+Enter"],
    description: "Save cell edits",
    category: "editing",
  },
  {
    id: "cell-edit-cancel",
    keys: ["Escape"],
    description: "Cancel cell edits",
    category: "editing",
  },

  // Modal shortcuts
  {
    id: "confirm-dialog-accept",
    keys: ["Enter"],
    description: "Confirm dialog action",
    category: "modal",
  },
  {
    id: "confirm-dialog-cancel",
    keys: ["Escape"],
    description: "Cancel dialog action",
    category: "modal",
  },

  // Query tab shortcuts
  {
    id: "query-mode-find",
    keys: ["Enter"],
    description: "Switch to Find mode",
    category: "query",
    context: "Query tab mode selector",
  },
  {
    id: "query-mode-aggregate",
    keys: ["Enter"],
    description: "Switch to Aggregate mode",
    category: "query",
    context: "Query tab mode selector",
  },
  {
    id: "query-mode-sql",
    keys: ["Enter"],
    description: "Switch to SQL mode",
    category: "query",
    context: "Query tab mode selector",
  },
  {
    id: "query-mode-update",
    keys: ["Enter"],
    description: "Switch to Update mode",
    category: "query",
    context: "Query tab mode selector",
  },
  {
    id: "query-mode-insert",
    keys: ["Enter"],
    description: "Switch to Insert mode",
    category: "query",
    context: "Query tab mode selector",
  },
  {
    id: "run-query",
    keys: ["CmdOrCtrl+Enter"],
    description: "Run the current query",
    category: "query",
    context: "Query tab",
  },
  {
    id: "save-connection",
    keys: ["CmdOrCtrl+S"],
    description: "Save connection or form",
    category: "editing",
    context: "Connection form, settings",
  },
  {
    id: "test-connection",
    keys: ["CmdOrCtrl+T"],
    description: "Test database connection",
    category: "global",
    context: "Connection form",
  },

  // Shell shortcuts
  {
    id: "shell-execute",
    keys: ["Enter"],
    description: "Execute shell command",
    category: "shell",
    context: "MongoDB shell",
  },
  {
    id: "shell-history-prev",
    keys: ["ArrowUp"],
    description: "Previous command in history",
    category: "shell",
    context: "MongoDB shell",
  },
  {
    id: "shell-history-next",
    keys: ["ArrowDown"],
    description: "Next command in history",
    category: "shell",
    context: "MongoDB shell",
  },

  // Data model shortcuts
  {
    id: "diagram-fit",
    keys: ["F"],
    description: "Fit diagram to view",
    category: "data-model",
    context: "Data model diagram",
  },
  {
    id: "diagram-relayout",
    keys: ["R"],
    description: "Re-layout diagram",
    category: "data-model",
    context: "Data model diagram",
  },
];

export const shortcutsByCategory = shortcuts.reduce((acc, shortcut) => {
  if (!acc[shortcut.category]) {
    acc[shortcut.category] = [];
  }
  acc[shortcut.category].push(shortcut);
  return acc;
}, {} as Record<ShortcutCategory, Shortcut[]>);

export function getShortcutDisplay(keys: string[]): string {
  return formatShortcut(keys.flatMap(parseShortcut));
}

export function getShortcutById(id: string): Shortcut | undefined {
  return shortcuts.find(s => s.id === id);
}

export function getShortcutsByCategory(category: ShortcutCategory): Shortcut[] {
  return shortcutsByCategory[category] || [];
}