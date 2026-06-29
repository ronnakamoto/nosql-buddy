import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ShortcutsMap } from "./ShortcutsMap";

describe("ShortcutsMap component", () => {
  it("should render when open", () => {
    const onClose = vi.fn();
    render(<ShortcutsMap open={true} onClose={onClose} />);
    
    expect(screen.getByRole("dialog", { name: "Keyboard shortcuts" })).toBeInTheDocument();
    expect(screen.getByText("Keyboard Shortcuts")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Search shortcuts...")).toBeInTheDocument();
  });

  it("should not render when closed", () => {
    const onClose = vi.fn();
    const { container } = render(<ShortcutsMap open={false} onClose={onClose} />);
    
    expect(container.firstChild).toBeNull();
  });

  it("should show all shortcuts by default", () => {
    const onClose = vi.fn();
    render(<ShortcutsMap open={true} onClose={onClose} />);
    
    // Should show some global shortcuts
    expect(screen.getByText("Open command palette")).toBeInTheDocument();
    expect(screen.getByText("Show keyboard shortcuts")).toBeInTheDocument();
    expect(screen.getByText("New connection")).toBeInTheDocument();
  });

  it("should filter shortcuts by search", () => {
    const onClose = vi.fn();
    render(<ShortcutsMap open={true} onClose={onClose} />);
    
    const searchInput = screen.getByPlaceholderText("Search shortcuts...");
    fireEvent.change(searchInput, { target: { value: "command palette" } });
    
    expect(screen.getByText("Open command palette")).toBeInTheDocument();
    // Should not show unrelated shortcuts
    expect(screen.queryByText("New connection")).not.toBeInTheDocument();
  });

  it("should filter shortcuts by category", () => {
    const onClose = vi.fn();
    render(<ShortcutsMap open={true} onClose={onClose} />);
    
    // Click the "Global Shortcuts" category button (not the section title)
    const globalCategoryButton = screen.getByRole("button", { name: /Global Shortcuts/ });
    fireEvent.click(globalCategoryButton);
    
    // Should show global shortcuts
    expect(screen.getByText("Open command palette")).toBeInTheDocument();
    expect(screen.getByText("Show keyboard shortcuts")).toBeInTheDocument();
  });

  it("should call onClose when close button is clicked", () => {
    const onClose = vi.fn();
    render(<ShortcutsMap open={true} onClose={onClose} />);
    
    const closeButton = screen.getByLabelText("Close shortcuts");
    fireEvent.click(closeButton);
    
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("should call onClose when backdrop is clicked", () => {
    const onClose = vi.fn();
    render(<ShortcutsMap open={true} onClose={onClose} />);
    
    const backdrop = screen.getByRole("dialog").firstChild as HTMLElement;
    fireEvent.click(backdrop);
    
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("should show empty state when no shortcuts match search", () => {
    const onClose = vi.fn();
    render(<ShortcutsMap open={true} onClose={onClose} />);
    
    const searchInput = screen.getByPlaceholderText("Search shortcuts...");
    fireEvent.change(searchInput, { target: { value: "nonexistent shortcut" } });
    
    expect(screen.getByText(/No shortcuts found matching/)).toBeInTheDocument();
  });

  it("should display keyboard shortcuts with proper formatting", () => {
    const onClose = vi.fn();
    render(<ShortcutsMap open={true} onClose={onClose} />);
    
    // Should show keyboard shortcuts in kbd elements
    const kbdElements = document.querySelectorAll(".kbd");
    expect(kbdElements.length).toBeGreaterThan(0);
    
    // Should contain common shortcuts
    const shortcutTexts = Array.from(kbdElements).map(el => el.textContent);
    expect(shortcutTexts.some(text => text?.includes("CmdOrCtrl+K"))).toBe(true);
    expect(shortcutTexts.some(text => text?.includes("CmdOrCtrl+N"))).toBe(true);
  });
});