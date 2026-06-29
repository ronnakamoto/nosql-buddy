/**
 * Pure history-navigation math for the Shell REPL editor.
 *
 * The Shell keeps an ordered list of previously-run scripts. Pressing the
 * Up arrow (when the caret is on the first line and no completion popup is
 * open) walks backwards through that list; Down walks forward and finally
 * clears the input.
 *
 * These helpers are intentionally free of React / CodeMirror so they can be
 * unit-tested in isolation. `cursor` is the index into the history list that
 * is currently shown, or `-1` when the user is composing a fresh command.
 */

/** Sentinel returned by {@link nextHistoryIndex} meaning "clear the input". */
export const HISTORY_CLEAR = -1 as const;

/**
 * Index to recall when pressing Up (older entry).
 *
 * Returns `null` when there is nothing to recall (empty history). When not
 * currently navigating (`cursor < 0`) it jumps to the most recent entry;
 * otherwise it steps one entry older, clamped at the oldest (index 0).
 */
export function prevHistoryIndex(length: number, cursor: number): number | null {
  if (length <= 0) return null;
  if (cursor < 0) return length - 1;
  return Math.max(0, cursor - 1);
}

/**
 * Index to recall when pressing Down (newer entry).
 *
 * Returns `null` when not navigating (`cursor < 0`). Steps one entry newer;
 * stepping past the newest entry returns {@link HISTORY_CLEAR} to signal the
 * caller should clear the input and stop navigating.
 */
export function nextHistoryIndex(
  length: number,
  cursor: number,
): number | typeof HISTORY_CLEAR | null {
  if (cursor < 0) return null;
  const next = cursor + 1;
  if (next >= length) return HISTORY_CLEAR;
  return next;
}
