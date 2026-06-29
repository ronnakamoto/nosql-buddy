import { describe, it, expect } from "vitest";
import { 
  shortcuts, 
  shortcutsByCategory, 
  getShortcutById, 
  getShortcutsByCategory,
  type ShortcutCategory 
} from "./shortcuts";

describe("shortcuts registry", () => {
  it("should have all required shortcuts", () => {
    expect(shortcuts.length).toBeGreaterThan(0);
    
    // Check for essential global shortcuts
    const globalShortcuts = getShortcutsByCategory("global");
    expect(globalShortcuts.some(s => s.id === "command-palette")).toBe(true);
    expect(globalShortcuts.some(s => s.id === "shortcuts-map")).toBe(true);
    expect(globalShortcuts.some(s => s.id === "new-connection")).toBe(true);
  });

  it("should categorize shortcuts correctly", () => {
    const categories = Object.keys(shortcutsByCategory);
    expect(categories).toContain("global");
    expect(categories).toContain("navigation");
    expect(categories).toContain("editing");
  });

  it("should find shortcuts by ID", () => {
    const commandPalette = getShortcutById("command-palette");
    expect(commandPalette).toBeDefined();
    expect(commandPalette?.keys).toContain("CmdOrCtrl+K");
    expect(commandPalette?.category).toBe("global");
  });

  it("should return undefined for unknown shortcut ID", () => {
    const unknown = getShortcutById("unknown-shortcut");
    expect(unknown).toBeUndefined();
  });

  it("should group shortcuts by category", () => {
    const globalShortcuts = getShortcutsByCategory("global");
    const queryShortcuts = getShortcutsByCategory("query");
    
    expect(globalShortcuts.length).toBeGreaterThan(0);
    expect(queryShortcuts.length).toBeGreaterThan(0);
    
    // All shortcuts in a category should have that category
    globalShortcuts.forEach(shortcut => {
      expect(shortcut.category).toBe("global");
    });
  });

  it("should have unique shortcut IDs", () => {
    const ids = shortcuts.map(s => s.id);
    const uniqueIds = new Set(ids);
    expect(ids.length).toBe(uniqueIds.size);
  });

  it("should have valid shortcut structure", () => {
    shortcuts.forEach(shortcut => {
      expect(shortcut.id).toBeDefined();
      expect(shortcut.id).toBeTruthy();
      expect(shortcut.keys).toBeDefined();
      expect(shortcut.keys.length).toBeGreaterThan(0);
      expect(shortcut.description).toBeDefined();
      expect(shortcut.description).toBeTruthy();
      expect(shortcut.category).toBeDefined();
      
      // Check that category is valid
      const validCategories: ShortcutCategory[] = [
        "global", "navigation", "editing", "modal", "query", "shell", "data-model"
      ];
      expect(validCategories).toContain(shortcut.category);
    });
  });
});