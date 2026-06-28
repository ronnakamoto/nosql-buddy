import { describe, it, expect } from "vitest";
import { getSuggestions, type EditorContext } from "./autocomplete";

const mockSchema = {
  topLevelFields: ["name", "price", "status", "category", "tags", "metadata"],
  allPaths: [
    "name",
    "price",
    "status",
    "category",
    "tags",
    "metadata",
    "metadata.createdAt",
    "metadata.updatedBy",
    "metadata.updatedBy.name",
  ],
  childrenByPrefix: new Map<string, string[]>([
    ["metadata", ["metadata.createdAt", "metadata.updatedBy"]],
    ["metadata.updatedBy", ["metadata.updatedBy.name"]],
  ]),
};

function suggest(
  text: string,
  offset: number,
  context: EditorContext,
  schema = mockSchema,
) {
  return getSuggestions(text, offset, context, schema).suggestions.map((s) => s.label);
}

function suggestSql(text: string, offset: number) {
  return getSuggestions(text, offset, "sql").suggestions.map((s) => s.label);
}

function suggestNoSchema(text: string, offset: number, context: EditorContext) {
  return getSuggestions(text, offset, context, undefined).suggestions.map((s) => s.label);
}

// ─── Filter operators ──────────────────────────────────────────────

describe("autocomplete - filter context", () => {
  it("suggests basic filter operators after $", () => {
    const labels = suggest('"$e', 3, "filter");
    expect(labels).toContain("$eq");
    expect(labels).toContain("$elemMatch");
    expect(labels).toContain("$expr");
    expect(labels).not.toContain("$set");
  });

  it("suggests geospatial operators", () => {
    const geoLabels = suggest('"$geo', 5, "filter");
    expect(geoLabels).toContain("$geoWithin");
    expect(geoLabels).toContain("$geoIntersects");

    const nearLabels = suggest('"$near', 5, "filter");
    expect(nearLabels).toContain("$near");
    expect(nearLabels).toContain("$nearSphere");
  });

  it("suggests $expr, $jsonSchema, $text, $where, $comment", () => {
    const labels = suggest('"$e', 3, "filter");
    expect(labels).toContain("$expr");

    const jsLabels = suggest('"$json', 6, "filter");
    expect(jsLabels).toContain("$jsonSchema");

    const textLabels = suggest('"$tex', 5, "filter");
    expect(textLabels).toContain("$text");

    const whereLabels = suggest('"$w', 3, "filter");
    expect(whereLabels).toContain("$where");

    const commentLabels = suggest('"$com', 5, "filter");
    expect(commentLabels).toContain("$comment");
  });

  it("suggests field names in object key position", () => {
    const labels = suggest('{ "na', 5, "filter");
    expect(labels).toContain("name");
    expect(labels).not.toContain("$eq");
  });

  it("does not suggest fields when typing a value", () => {
    const labels = suggest('{ "name": "val', 13, "filter");
    expect(labels).toHaveLength(0);
  });

  it("suggests nested field paths under a parent", () => {
    const labels = suggest('{ "metadata": { "createdAt": 1, "u', 32, "filter");
    expect(labels).toContain("updatedBy");
  });

  it("suggests BSON type values inside $type", () => {
    const labels = suggest('{ "$type": "', 12, "filter");
    expect(labels).toContain("object");
    expect(labels).toContain("objectId");
    expect(labels).toContain("bool");
    expect(labels).toContain("string");
    expect(labels).toContain("date");
  });

  it("suggests date tag values inside string values", () => {
    const labels = suggest('{ "createdAt": "#', 16, "filter");
    expect(labels).toContain("#today");
    expect(labels).toContain("#now");
    expect(labels).toContain("#tomorrow");
  });
});

// ─── Update operators ──────────────────────────────────────────────

describe("autocomplete - update context", () => {
  it("suggests update operators", () => {
    const labels = suggest('"$se', 4, "update");
    expect(labels).toContain("$set");
    expect(labels).not.toContain("$match");
  });

  it("suggests $setOnInsert, $pullAll, $each", () => {
    const labels = suggest('"$set', 5, "update");
    expect(labels).toContain("$set");
    expect(labels).toContain("$setOnInsert");

    const pullLabels = suggest('"$pull', 6, "update");
    expect(pullLabels).toContain("$pull");
    expect(pullLabels).toContain("$pullAll");

    const eachLabels = suggest('"$e', 3, "update");
    expect(eachLabels).toContain("$each");
  });

  it("suggests field names inside $set", () => {
    const labels = suggest('{ "$set": { "pri', 15, "update");
    expect(labels).toContain("price");
  });

  it("suggests date tags inside update values", () => {
    const labels = suggest('{ "$set": { "updatedAt": "#', 28, "update");
    expect(labels).toContain("#now");
    expect(labels).toContain("#today");
  });
});

// ─── Aggregation stages ────────────────────────────────────────────

describe("autocomplete - aggregate stages", () => {
  it("suggests stage operators for top-level pipeline", () => {
    const labels = suggest('[ { "$ma', 8, "aggregate");
    expect(labels).toContain("$match");
    expect(labels).not.toContain("$sum");
  });

  it("suggests all new stages", () => {
    const allLabels = suggest('[ { "$', 6, "aggregate");
    expect(allLabels).toContain("$sample");
    expect(allLabels).toContain("$replaceRoot");
    expect(allLabels).toContain("$replaceWith");
    expect(allLabels).toContain("$set");
    expect(allLabels).toContain("$unset");
    expect(allLabels).toContain("$sortByCount");
    expect(allLabels).toContain("$graphLookup");
    expect(allLabels).toContain("$geoNear");
    expect(allLabels).toContain("$densify");
    expect(allLabels).toContain("$fill");
    expect(allLabels).toContain("$documents");
    expect(allLabels).toContain("$redact");
  });
});

// ─── Aggregation expressions ───────────────────────────────────────

describe("autocomplete - aggregate expressions", () => {
  it("suggests expression operators inside a stage body", () => {
    const labels = suggest('{ "total": " $sum', 17, "aggregate");
    expect(labels).toContain("$sum");
    expect(labels).not.toContain("$match");
  });

  it("suggests string expressions", () => {
    const labels = suggest('{ "$group": { "x": "$con', 24, "aggregate");
    expect(labels).toContain("$concat");
    expect(labels).toContain("$cond");
  });

  it("suggests array expressions", () => {
    const labels = suggest('{ "$group": { "x": "$map', 24, "aggregate");
    expect(labels).toContain("$map");

    const mergeLabels = suggest('{ "$group": { "x": "$mer', 25, "aggregate");
    expect(mergeLabels).toContain("$mergeObjects");
  });

  it("suggests date expressions", () => {
    const labels = suggest('{ "$group": { "x": "$date', 24, "aggregate");
    expect(labels).toContain("$dateToString");
    expect(labels).toContain("$dateAdd");
    expect(labels).toContain("$dateDiff");
    expect(labels).toContain("$dateTrunc");
  });

  it("suggests math expressions", () => {
    const labels = suggest('{ "$group": { "x": "$a', 23, "aggregate");
    expect(labels).toContain("$abs");
    expect(labels).toContain("$add");

    const ceilLabels = suggest('{ "$group": { "x": "$ceil', 26, "aggregate");
    expect(ceilLabels).toContain("$ceil");

    const sqrtLabels = suggest('{ "$group": { "x": "$sqrt', 26, "aggregate");
    expect(sqrtLabels).toContain("$sqrt");

    const roundLabels = suggest('{ "$group": { "x": "$round', 27, "aggregate");
    expect(roundLabels).toContain("$round");
  });

  it("suggests boolean expressions", () => {
    const labels = suggest('{ "$group": { "x": "$and', 25, "aggregate");
    expect(labels).toContain("$and");

    const orLabels = suggest('{ "$group": { "x": "$or', 24, "aggregate");
    expect(orLabels).toContain("$or");

    const notLabels = suggest('{ "$group": { "x": "$not', 26, "aggregate");
    expect(notLabels).toContain("$not");
  });

  it("suggests type conversion expressions", () => {
    const labels = suggest('{ "$group": { "x": "$to', 24, "aggregate");
    expect(labels).toContain("$toBool");
    expect(labels).toContain("$toString");
    expect(labels).toContain("$toDate");
    expect(labels).toContain("$toObjectId");
    expect(labels).toContain("$toLong");

    const convertLabels = suggest('{ "$group": { "x": "$convert', 29, "aggregate");
    expect(convertLabels).toContain("$convert");
  });

  it("suggests set expressions", () => {
    const labels = suggest('{ "$group": { "x": "$set', 24, "aggregate");
    expect(labels).toContain("$setEquals");
    expect(labels).toContain("$setDifference");
    expect(labels).toContain("$setIntersection");
    expect(labels).toContain("$setUnion");
  });

  it("suggests misc expressions", () => {
    const labels = suggest('{ "$group": { "x": "$ra', 25, "aggregate");
    expect(labels).toContain("$rand");
    expect(labels).toContain("$range");

    const regexLabels = suggest('{ "$group": { "x": "$re', 25, "aggregate");
    expect(regexLabels).toContain("$regexMatch");
    expect(regexLabels).toContain("$reduce");

    const revLabels = suggest('{ "$group": { "x": "$reverse', 30, "aggregate");
    expect(revLabels).toContain("$reverseArray");
  });

  it("suggests conditional expressions", () => {
    const labels = suggest('{ "$group": { "x": "$switch', 29, "aggregate");
    expect(labels).toContain("$switch");

    const sliceLabels = suggest('{ "$group": { "x": "$slice', 28, "aggregate");
    expect(sliceLabels).toContain("$slice");
  });
});

// ─── SQL keywords ────────────────────────────────────────────────────

describe("autocomplete - SQL context", () => {
  it("suggests SQL keywords", () => {
    const labels = suggestSql("SEL", 3);
    expect(labels).toContain("SELECT");
    expect(labels).not.toContain("FROM");
  });

  it("suggests multi-word keywords", () => {
    const labels = suggestSql("ORDER B", 7);
    expect(labels).toContain("ORDER BY");
  });

  it("suggests new join keywords", () => {
    expect(suggestSql("RIGHT", 5)).toContain("RIGHT JOIN");
    expect(suggestSql("FULL", 4)).toContain("FULL JOIN");
    expect(suggestSql("CROSS", 5)).toContain("CROSS JOIN");
  });

  it("suggests CASE WHEN / THEN / ELSE / END", () => {
    expect(suggestSql("CASE", 4)).toContain("CASE WHEN");
    expect(suggestSql("TH", 2)).toContain("THEN");
    expect(suggestSql("ELS", 3)).toContain("ELSE");
  });

  it("suggests UNION and UNION ALL", () => {
    const labels = suggestSql("UNION", 5);
    expect(labels).toContain("UNION");
    expect(labels).toContain("UNION ALL");
  });

  it("suggests BETWEEN, WITH, CAST, CONCAT", () => {
    expect(suggestSql("BET", 3)).toContain("BETWEEN");
    expect(suggestSql("WI", 2)).toContain("WITH");
    expect(suggestSql("CAS", 3)).toContain("CAST");
    expect(suggestSql("CON", 3)).toContain("CONCAT");
  });

  it("suggests date/time functions", () => {
    expect(suggestSql("NOW", 3)).toContain("NOW()");
    expect(suggestSql("CURRENT_D", 9)).toContain("CURRENT_DATE");
    expect(suggestSql("CURRENT_T", 9)).toContain("CURRENT_TIMESTAMP");
  });

  it("suggests TRUE, FALSE, NULL", () => {
    expect(suggestSql("TR", 2)).toContain("TRUE");
    expect(suggestSql("FA", 2)).toContain("FALSE");
    expect(suggestSql("NU", 2)).toContain("NULL");
  });
});

// ─── Insert context ────────────────────────────────────────────────

describe("autocomplete - insert context", () => {
  it("suggests field names for document keys", () => {
    const labels = suggest('{ "na', 5, "insert");
    expect(labels).toContain("name");
  });
});

// ─── Without schema ──────────────────────────────────────────────────

describe("autocomplete - without schema", () => {
  it("still suggests operators", () => {
    const labels = suggestNoSchema('"$eq', 4, "filter");
    expect(labels).toContain("$eq");
  });

  it("returns empty for field context without schema", () => {
    const labels = suggestNoSchema('{ "x', 4, "filter");
    expect(labels).toHaveLength(0);
  });
});

// ─── Edge cases ──────────────────────────────────────────────────────

describe("autocomplete - edge cases", () => {
  it("handles cursor at end of text", () => {
    const labels = suggest('{"$in', 5, "filter");
    expect(labels).toContain("$in");
  });

  it("handles empty string", () => {
    const labels = suggest("", 0, "filter");
    expect(labels).toContain("ObjectId filter");
  });

  it("limits results to 50 suggestions", () => {
    const result = getSuggestions("", 0, "filter", mockSchema);
    expect(result.suggestions.length).toBeLessThanOrEqual(50);
  });

  it("ranks exact prefix matches first", () => {
    const result = getSuggestions('"$eq', 4, "filter", mockSchema);
    expect(result.suggestions[0].label).toBe("$eq");
  });

  it("suggests nothing for non-key positions with plain prefix", () => {
    const result = getSuggestions('{ "name": "abc', 14, "filter", mockSchema);
    expect(result.suggestions.length).toBe(0);
  });
});

// ─── Catalog completeness ────────────────────────────────────────────

describe("autocomplete - catalog completeness", () => {
  it("has at least 25 filter operators", () => {
    const result = getSuggestions('"$', 2, "filter");
    expect(result.suggestions.length).toBeGreaterThanOrEqual(25);
  });

  it("has at least 15 update operators", () => {
    const result = getSuggestions('"$', 2, "update");
    expect(result.suggestions.length).toBeGreaterThanOrEqual(15);
  });

  it("has at least 25 stage operators", () => {
    const result = getSuggestions('"$', 2, "aggregate");
    expect(result.suggestions.length).toBeGreaterThanOrEqual(25);
  });

  it("has at least 50 expression operators", () => {
    const result = getSuggestions('{ "x": "$m', 9, "aggregate");
    expect(result.suggestions.length).toBeGreaterThanOrEqual(50);
  });

  it("has at least 45 SQL keywords", () => {
    const result = getSuggestions("", 0, "sql");
    expect(result.suggestions.length).toBeGreaterThanOrEqual(45);
  });

  it("has 21 BSON type values", () => {
    const result = getSuggestions('{ "$type": "', 12, "filter");
    expect(result.suggestions.length).toBeGreaterThanOrEqual(21);
  });

  it("has 8 date tag values", () => {
    const result = getSuggestions('{ "x": "#', 8, "filter");
    expect(result.suggestions.length).toBeGreaterThanOrEqual(8);
  });
});
