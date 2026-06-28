import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { InsertDocumentModal } from "./InsertDocumentModal";
import { ToastProvider } from "../context/ToastContext";

const mockInvoke = vi.fn();
vi.mock("../ipc/commands", () => ({
  __esModule: true,
  default: {
    insertDocument: (...args: unknown[]) => mockInvoke("insertDocument", ...args),
    insertManyDocuments: (...args: unknown[]) => mockInvoke("insertManyDocuments", ...args),
  },
  formatError: (err: unknown) => String(err),
}));

const mockToastApi = {
  push: vi.fn(),
  pushToast: vi.fn(),
};

function renderWithToast(ui: React.ReactNode) {
  return render(<ToastProvider value={mockToastApi}>{ui}</ToastProvider>);
}

describe("InsertDocumentModal", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockToastApi.push.mockReset();
    mockToastApi.pushToast.mockReset();
  });

  it("renders single-document mode by default", () => {
    renderWithToast(
      <InsertDocumentModal
        open
        connectionId="c1"
        database="db"
        collection="coll"
        onClose={() => {}}
        onInserted={() => {}}
        onError={() => {}}
      />,
    );
    expect(screen.getByText("Insert document into db.coll")).toBeInTheDocument();
    expect(screen.getByLabelText("Document JSON")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Insert" })).toBeInTheDocument();
  });

  it("renders insert-many mode when many=true", () => {
    renderWithToast(
      <InsertDocumentModal
        open
        connectionId="c1"
        database="db"
        collection="coll"
        many
        onClose={() => {}}
        onInserted={() => {}}
        onError={() => {}}
      />,
    );
    expect(screen.getByText("Insert documents into db.coll")).toBeInTheDocument();
    expect(screen.getByLabelText("Documents JSON")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Insert Many" })).toBeInTheDocument();
  });

  it("calls insertDocument with parsed JSON on submit", async () => {
    mockInvoke.mockResolvedValue("abc123");
    const onInserted = vi.fn();
    const onClose = vi.fn();

    renderWithToast(
      <InsertDocumentModal
        open
        connectionId="c1"
        database="db"
        collection="coll"
        onClose={onClose}
        onInserted={onInserted}
        onError={() => {}}
      />,
    );

    const textarea = screen.getByLabelText("Document JSON");
    fireEvent.change(textarea, { target: { value: '{"name":"Test"}' } });
    fireEvent.click(screen.getByRole("button", { name: "Insert" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("insertDocument", {
        connectionId: "c1",
        database: "db",
        collection: "coll",
        documentJson: '{"name":"Test"}',
      });
    });
    expect(onInserted).toHaveBeenCalledWith("abc123");
    expect(onClose).toHaveBeenCalled();
  });

  it("calls insertManyDocuments with parsed array on submit", async () => {
    mockInvoke.mockResolvedValue({ insertedCount: 2, insertedIds: ["a", "b"] });
    const onInserted = vi.fn();
    const onClose = vi.fn();

    renderWithToast(
      <InsertDocumentModal
        open
        connectionId="c1"
        database="db"
        collection="coll"
        many
        onClose={onClose}
        onInserted={onInserted}
        onError={() => {}}
      />,
    );

    const textarea = screen.getByLabelText("Documents JSON");
    fireEvent.change(textarea, { target: { value: '[{"x":1},{"x":2}]' } });
    fireEvent.click(screen.getByRole("button", { name: "Insert Many" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("insertManyDocuments", {
        connectionId: "c1",
        database: "db",
        collection: "coll",
        documentsJson: '[{"x":1},{"x":2}]',
      });
    });
    expect(onInserted).toHaveBeenCalledWith("a, b");
    expect(onClose).toHaveBeenCalled();
  });

  it("rejects non-object JSON in single mode", async () => {
    const onError = vi.fn();
    renderWithToast(
      <InsertDocumentModal
        open
        connectionId="c1"
        database="db"
        collection="coll"
        onClose={() => {}}
        onInserted={() => {}}
        onError={onError}
      />,
    );

    const textarea = screen.getByLabelText("Document JSON");
    fireEvent.change(textarea, { target: { value: '[1,2,3]' } });
    fireEvent.click(screen.getByRole("button", { name: "Insert" }));

    await waitFor(() => {
      expect(mockToastApi.push).toHaveBeenCalledWith("Document must be a JSON object.", "error");
    });
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("rejects non-array JSON in many mode", async () => {
    const onError = vi.fn();
    renderWithToast(
      <InsertDocumentModal
        open
        connectionId="c1"
        database="db"
        collection="coll"
        many
        onClose={() => {}}
        onInserted={() => {}}
        onError={onError}
      />,
    );

    const textarea = screen.getByLabelText("Documents JSON");
    fireEvent.change(textarea, { target: { value: '{"x":1}' } });
    fireEvent.click(screen.getByRole("button", { name: "Insert Many" }));

    await waitFor(() => {
      expect(mockToastApi.push).toHaveBeenCalledWith(
        "Insert many requires a JSON array of objects.",
        "error",
      );
    });
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
