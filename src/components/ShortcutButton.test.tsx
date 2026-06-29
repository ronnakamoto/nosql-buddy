import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ShortcutButton } from "./ShortcutButton";

describe("ShortcutButton component", () => {
  it("should render without shortcut", () => {
    render(<ShortcutButton>Click me</ShortcutButton>);
    
    const button = screen.getByRole("button", { name: "Click me" });
    expect(button).toBeInTheDocument();
    expect(button).toHaveClass("shortcut-btn", "shortcut-btn--default", "shortcut-btn--md");
    
    // Should not have kbd element
    expect(button.querySelector(".shortcut-btn__kbd")).not.toBeInTheDocument();
  });

  it("should render with string shortcut", () => {
    render(<ShortcutButton shortcut="CmdOrCtrl+K">Save</ShortcutButton>);
    
    const button = screen.getByRole("button");
    const kbd = button.querySelector(".shortcut-btn__kbd");
    
    expect(button).toBeInTheDocument();
    expect(kbd).toBeInTheDocument();
    // Should format based on platform (Mac or non-Mac)
    expect(kbd!.textContent).toMatch(/^(⌘K|Ctrl\+K)$/);
  });

  it("should render with ShortcutKeys array", () => {
    const shortcutKeys = [{ ctrl: true, key: "S" }];
    render(<ShortcutButton shortcut={shortcutKeys}>Save</ShortcutButton>);
    
    const button = screen.getByRole("button");
    const kbd = button.querySelector(".shortcut-btn__kbd");
    
    expect(button).toBeInTheDocument();
    expect(kbd).toBeInTheDocument();
    // Should format based on platform (Mac or non-Mac)
    expect(kbd!.textContent).toMatch(/^(⌃S|Ctrl\+S)$/);
  });

  it("should apply variant classes correctly", () => {
    const { rerender } = render(<ShortcutButton variant="primary">Primary</ShortcutButton>);
    let button = screen.getByRole("button");
    expect(button).toHaveClass("shortcut-btn--primary");

    rerender(<ShortcutButton variant="ghost">Ghost</ShortcutButton>);
    button = screen.getByRole("button");
    expect(button).toHaveClass("shortcut-btn--ghost");

    rerender(<ShortcutButton variant="danger">Danger</ShortcutButton>);
    button = screen.getByRole("button");
    expect(button).toHaveClass("shortcut-btn--danger");
  });

  it("should apply size classes correctly", () => {
    const { rerender } = render(<ShortcutButton size="sm">Small</ShortcutButton>);
    let button = screen.getByRole("button");
    expect(button).toHaveClass("shortcut-btn--sm");

    rerender(<ShortcutButton size="lg">Large</ShortcutButton>);
    button = screen.getByRole("button");
    expect(button).toHaveClass("shortcut-btn--lg");
  });

  it("should handle click events", () => {
    const handleClick = vi.fn();
    render(<ShortcutButton onClick={handleClick}>Click me</ShortcutButton>);
    
    const button = screen.getByRole("button");
    fireEvent.click(button);
    
    expect(handleClick).toHaveBeenCalledTimes(1);
  });

  it("should pass through other props", () => {
    render(
      <ShortcutButton disabled data-testid="test-button" title="Test button">
        Button
      </ShortcutButton>
    );
    
    const button = screen.getByRole("button");
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("data-testid", "test-button");
    expect(button).toHaveAttribute("title", "Test button");
  });

  it("should combine custom className with default classes", () => {
    render(<ShortcutButton className="custom-class">Button</ShortcutButton>);
    
    const button = screen.getByRole("button");
    expect(button).toHaveClass("shortcut-btn", "shortcut-btn--default", "shortcut-btn--md", "custom-class");
  });

  it("should format complex shortcuts correctly", () => {
    render(<ShortcutButton shortcut="CmdOrCtrl+Shift+Z">Redo</ShortcutButton>);
    
    const button = screen.getByRole("button");
    const kbd = button.querySelector(".shortcut-btn__kbd");
    
    expect(kbd).toBeInTheDocument();
    // Should format based on platform (Mac or non-Mac)
    expect(kbd!.textContent).toMatch(/^(⌘⇧Z|Ctrl\+Shift\+Z)$/);
  });

  it("should handle multiple alternative shortcuts", () => {
    render(<ShortcutButton shortcut="CmdOrCtrl+S or CmdOrCtrl+Enter">Save</ShortcutButton>);
    
    const button = screen.getByRole("button");
    const kbd = button.querySelector(".shortcut-btn__kbd");
    
    expect(kbd).toBeInTheDocument();
    // Should format based on platform (Mac or non-Mac)
    expect(kbd!.textContent).toMatch(/^(⌘S or ⌘↵|Ctrl\+S or Ctrl\+↵)$/);
  });
});