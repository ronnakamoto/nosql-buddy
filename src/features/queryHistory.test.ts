import {
  type QueryMode,
  modeLabel,
  fileExtension,
  fileFilter,
  listHistory,
  pushHistory,
  clearHistory,
  saveBookmark,
  getBookmark,
  listBookmarks,
  deleteBookmark,
} from "./queryHistory";

describe("queryHistory", () => {
  const conn = "conn1";
  const db = "testdb";
  const coll = "testcoll";

  beforeEach(() => {
    localStorage.clear();
  });

  describe("modeLabel", () => {
    it.each([
      ["find", "Find"],
      ["aggregate", "Aggregation"],
      ["sql", "SQL"],
      ["update", "Update"],
      ["insert", "Insert"],
    ] as [QueryMode, string][])('returns "%s" for mode %s', (mode, expected) => {
      expect(modeLabel(mode)).toBe(expected);
    });
  });

  describe("fileExtension", () => {
    it('returns "sql" for sql mode', () => {
      expect(fileExtension("sql")).toBe("sql");
    });

    it.each(["find", "aggregate", "update", "insert"] as QueryMode[])(
      'returns "json" for %s mode',
      (mode) => {
        expect(fileExtension(mode)).toBe("json");
      },
    );
  });

  describe("fileFilter", () => {
    it('returns SQL filter for sql mode', () => {
      expect(fileFilter("sql")).toEqual([
        { name: "SQL", extensions: ["sql"] },
      ]);
    });

    it.each(["find", "aggregate", "update", "insert"] as QueryMode[])(
      'returns JSON filter for %s mode',
      (mode) => {
        expect(fileFilter(mode)).toEqual([
          { name: "JSON", extensions: ["json"] },
        ]);
      },
    );
  });

  describe("history CRUD", () => {
    it("stores and retrieves history for update mode", () => {
      pushHistory(conn, db, coll, "update", {
        ts: 1000,
        text: '{"filter":{},"update":{}}',
        durationMs: 50,
        docCount: 0,
        errored: false,
      });
      const history = listHistory(conn, db, coll, "update");
      expect(history).toHaveLength(1);
      expect(history[0].text).toBe('{"filter":{},"update":{}}');
    });

    it("stores and retrieves history for insert mode", () => {
      pushHistory(conn, db, coll, "insert", {
        ts: 2000,
        text: '{"name":"A"}',
        durationMs: 20,
        docCount: 1,
        errored: false,
      });
      const history = listHistory(conn, db, coll, "insert");
      expect(history).toHaveLength(1);
      expect(history[0].docCount).toBe(1);
    });

    it("dedupes consecutive identical entries", () => {
      const entry = {
        ts: 1000,
        text: "same",
        durationMs: 10,
        docCount: 1,
        errored: false,
      };
      pushHistory(conn, db, coll, "update", entry);
      pushHistory(conn, db, coll, "update", { ...entry, ts: 2000 });
      expect(listHistory(conn, db, coll, "update")).toHaveLength(1);
    });

    it("caps history at 20 entries", () => {
      for (let i = 0; i < 25; i++) {
        pushHistory(conn, db, coll, "find", {
          ts: i,
          text: `q${i}`,
          durationMs: 1,
          docCount: 1,
          errored: false,
        });
      }
      expect(listHistory(conn, db, coll, "find")).toHaveLength(20);
    });

    it("clears history by mode", () => {
      pushHistory(conn, db, coll, "find", {
        ts: 1,
        text: "a",
        durationMs: 1,
        docCount: 1,
        errored: false,
      });
      clearHistory(conn, db, coll, "find");
      expect(listHistory(conn, db, coll, "find")).toHaveLength(0);
    });
  });

  describe("bookmarks", () => {
    it("saves and loads bookmarks for update mode", () => {
      saveBookmark(conn, db, coll, "update", "bulk-status", '{"filter":{},"update":{"$set":{"s":1}}}');
      const bm = getBookmark(conn, db, coll, "update", "bulk-status");
      expect(bm).not.toBeNull();
      expect(bm!.text).toBe('{"filter":{},"update":{"$set":{"s":1}}}');
    });

    it("lists bookmarks across modes independently", () => {
      saveBookmark(conn, db, coll, "find", "f1", "{}");
      saveBookmark(conn, db, coll, "update", "u1", "{}");
      expect(listBookmarks(conn, db, coll, "find")).toHaveLength(1);
      expect(listBookmarks(conn, db, coll, "update")).toHaveLength(1);
    });

    it("deletes a bookmark", () => {
      saveBookmark(conn, db, coll, "insert", "tmpl", "{}");
      deleteBookmark(conn, db, coll, "insert", "tmpl");
      expect(getBookmark(conn, db, coll, "insert", "tmpl")).toBeNull();
    });
  });
});
