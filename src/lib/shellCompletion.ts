import type {
  Completion,
  CompletionContext,
  CompletionResult,
} from "@codemirror/autocomplete";
import type { AutocompleteResponse, CompletionItem } from "../ipc/commands";

export interface ShellCompletionRequest {
  textBeforeCursor: string;
}

export interface ShellCompletionOptions {
  request: (request: ShellCompletionRequest) => Promise<AutocompleteResponse>;
}

function completionType(detail: string): Completion["type"] {
  const lower = detail.toLowerCase();
  if (lower.includes("collection") || lower.includes("field")) return "property";
  if (lower.includes("method") || lower.includes("function")) return "function";
  if (lower.includes("operator") || lower.includes("global")) return "keyword";
  return "text";
}

export function itemToCompletion(item: CompletionItem): Completion {
  return {
    label: item.label,
    detail: item.detail,
    type: completionType(item.detail),
    apply: item.label,
  };
}

/**
 * Build the CodeMirror completion source for the mongosh editor.
 *
 * The backend already understands DB/collection/method/operator context and
 * returns filtered suggestions for the text before the cursor. CodeMirror
 * owns popup rendering, keyboard navigation, cancellation, and stale async
 * request handling.
 */
export function makeShellCompletionSource({
  request,
}: ShellCompletionOptions) {
  return async (cc: CompletionContext): Promise<CompletionResult | null> => {
    if (cc.explicit === false) {
      const before = cc.matchBefore(/[\w$]*$/);
      const prev = cc.pos > 0 ? cc.state.sliceDoc(cc.pos - 1, cc.pos) : "";
      const typedIdentifier = Boolean(before && before.text.length > 0);
      const afterTrigger = prev === "." || prev === "$";
      if (!typedIdentifier && !afterTrigger) return null;
    }

    const textBeforeCursor = cc.state.sliceDoc(0, cc.pos);
    const word = cc.matchBefore(/[\w$]*$/);
    const resp = await request({ textBeforeCursor });
    if (cc.aborted || resp.kind.kind === "none" || resp.items.length === 0) {
      return null;
    }

    return {
      from: word ? word.from : cc.pos,
      to: cc.pos,
      filter: false,
      options: resp.items.map(itemToCompletion),
    };
  };
}
