/**
 * Autocomplete engine for MongoDB query editors.
 *
 * Given the full document text, cursor offset, and an optional schema,
 * produces a ranked list of suggestions and a replacement span.
 */

export interface Suggestion {
  label: string;
  insertText: string;
  detail?: string;
  kind: "field" | "operator" | "keyword" | "value" | "snippet";
}

export interface CompletionResult {
  suggestions: Suggestion[];
  /** Start offset in the document of the text to replace. */
  replaceStart: number;
  /** End offset (same as cursor position). */
  replaceEnd: number;
}

/** MongoDB filter operators. */
const FILTER_OPERATORS: Suggestion[] = [
  { label: "$eq", insertText: '"$eq": ', kind: "operator", detail: "Matches values equal to" },
  { label: "$ne", insertText: '"$ne": ', kind: "operator", detail: "Matches values not equal to" },
  { label: "$gt", insertText: '"$gt": ', kind: "operator", detail: "Greater than" },
  { label: "$gte", insertText: '"$gte": ', kind: "operator", detail: "Greater than or equal" },
  { label: "$lt", insertText: '"$lt": ', kind: "operator", detail: "Less than" },
  { label: "$lte", insertText: '"$lte": ', kind: "operator", detail: "Less than or equal" },
  { label: "$in", insertText: '"$in": [', kind: "operator", detail: "Matches any value in array" },
  { label: "$nin", insertText: '"$nin": [', kind: "operator", detail: "Matches no value in array" },
  { label: "$exists", insertText: '"$exists": true', kind: "operator", detail: "Field exists" },
  { label: "$regex", insertText: '"$regex": "', kind: "operator", detail: "Regular expression" },
  { label: "$type", insertText: '"$type": "string"', kind: "operator", detail: "BSON type check" },
  { label: "$and", insertText: '"$and": [\n  {},\n  {}\n]', kind: "operator", detail: "Logical AND" },
  { label: "$or", insertText: '"$or": [\n  {},\n  {}\n]', kind: "operator", detail: "Logical OR" },
  { label: "$nor", insertText: '"$nor": [\n  {},\n  {}\n]', kind: "operator", detail: "Logical NOR" },
  { label: "$not", insertText: '"$not": {}', kind: "operator", detail: "Logical NOT" },
  { label: "$all", insertText: '"$all": [', kind: "operator", detail: "Array contains all" },
  { label: "$elemMatch", insertText: '"$elemMatch": {}', kind: "operator", detail: "Match array element" },
  { label: "$size", insertText: '"$size": 0', kind: "operator", detail: "Array length" },
  { label: "$mod", insertText: '"$mod": [2, 0]', kind: "operator", detail: "Modulo" },
  { label: "$expr", insertText: '"$expr": {}', kind: "operator", detail: "Aggregation expression in query" },
  { label: "$jsonSchema", insertText: '"$jsonSchema": {}', kind: "operator", detail: "JSON Schema validation" },
  { label: "$text", insertText: '"$text": { "$search": "" }', kind: "operator", detail: "Text search" },
  { label: "$where", insertText: '"$where": "function() { return true; }"', kind: "operator", detail: "JavaScript expression" },
  { label: "$comment", insertText: '"$comment": ""', kind: "operator", detail: "Query comment for profiling" },
  { label: "$geoWithin", insertText: '"$geoWithin": {}', kind: "operator", detail: "Geospatial within shape" },
  { label: "$geoIntersects", insertText: '"$geoIntersects": {}', kind: "operator", detail: "Geospatial intersection" },
  { label: "$near", insertText: '"$near": { "$geometry": { "type": "Point", "coordinates": [0, 0] } }', kind: "operator", detail: "Geospatial proximity" },
  { label: "$nearSphere", insertText: '"$nearSphere": { "$geometry": { "type": "Point", "coordinates": [0, 0] } }', kind: "operator", detail: "Spherical proximity" },
];

/** MongoDB update operators. */
const UPDATE_OPERATORS: Suggestion[] = [
  { label: "$set", insertText: '"$set": {\n  \n}', kind: "operator", detail: "Set field value" },
  { label: "$unset", insertText: '"$unset": {\n  "": 1\n}', kind: "operator", detail: "Remove field" },
  { label: "$inc", insertText: '"$inc": {\n  "": 1\n}', kind: "operator", detail: "Increment field" },
  { label: "$mul", insertText: '"$mul": {\n  "": 1\n}', kind: "operator", detail: "Multiply field" },
  { label: "$push", insertText: '"$push": {\n  "": \n}', kind: "operator", detail: "Add to array" },
  { label: "$pull", insertText: '"$pull": {\n  "": {}\n}', kind: "operator", detail: "Remove from array" },
  { label: "$addToSet", insertText: '"$addToSet": {\n  "": \n}', kind: "operator", detail: "Add unique to array" },
  { label: "$pop", insertText: '"$pop": {\n  "": 1\n}', kind: "operator", detail: "Remove first/last array element" },
  { label: "$rename", insertText: '"$rename": {\n  "": ""\n}', kind: "operator", detail: "Rename field" },
  { label: "$min", insertText: '"$min": {\n  "": \n}', kind: "operator", detail: "Update if smaller" },
  { label: "$max", insertText: '"$max": {\n  "": \n}', kind: "operator", detail: "Update if larger" },
  { label: "$currentDate", insertText: '"$currentDate": {\n  "": true\n}', kind: "operator", detail: "Set to current date" },
  { label: "$bit", insertText: '"$bit": {\n  "": { "and": 1 }\n}', kind: "operator", detail: "Bitwise update" },
  { label: "$setOnInsert", insertText: '"$setOnInsert": {\n  "": \n}', kind: "operator", detail: "Set only on upsert insert" },
  { label: "$pullAll", insertText: '"$pullAll": {\n  "": []\n}', kind: "operator", detail: "Remove all matching values" },
  { label: "$each", insertText: '"$each": []', kind: "operator", detail: "Modifier for $push / $addToSet" },
];

/** Aggregation stage operators. */
const STAGE_OPERATORS: Suggestion[] = [
  { label: "$match", insertText: '{ "$match": {} }', kind: "operator", detail: "Filter documents" },
  { label: "$project", insertText: '{ "$project": {} }', kind: "operator", detail: "Reshape documents" },
  { label: "$group", insertText: '{ "$group": { "_id": "$" } }', kind: "operator", detail: "Group by expression" },
  { label: "$sort", insertText: '{ "$sort": { "": 1 } }', kind: "operator", detail: "Sort documents" },
  { label: "$limit", insertText: '{ "$limit": 10 }', kind: "operator", detail: "Limit documents" },
  { label: "$skip", insertText: '{ "$skip": 10 }', kind: "operator", detail: "Skip documents" },
  { label: "$lookup", insertText: '{ "$lookup": { "from": "", "localField": "", "foreignField": "", "as": "" } }', kind: "operator", detail: "Left join" },
  { label: "$unwind", insertText: '{ "$unwind": "$" }', kind: "operator", detail: "Deconstruct array" },
  { label: "$addFields", insertText: '{ "$addFields": {} }', kind: "operator", detail: "Add new fields" },
  { label: "$facet", insertText: '{ "$facet": {} }', kind: "operator", detail: "Multi-facet aggregation" },
  { label: "$bucket", insertText: '{ "$bucket": { "groupBy": "$", "boundaries": [] } }', kind: "operator", detail: "Bucket by boundaries" },
  { label: "$out", insertText: '{ "$out": "" }', kind: "operator", detail: "Write to collection" },
  { label: "$merge", insertText: '{ "$merge": { "into": "" } }', kind: "operator", detail: "Merge into collection" },
  { label: "$count", insertText: '{ "$count": "" }', kind: "operator", detail: "Count documents" },
  { label: "$sample", insertText: '{ "$sample": { "size": 10 } }', kind: "operator", detail: "Random sample" },
  { label: "$redact", insertText: '{ "$redact": { "$cond": { "if": {}, "then": "$$DESCEND", "else": "$$PRUNE" } } }', kind: "operator", detail: "Restrict content" },
  { label: "$replaceRoot", insertText: '{ "$replaceRoot": { "newRoot": "$" } }', kind: "operator", detail: "Promote embedded doc to root" },
  { label: "$replaceWith", insertText: '{ "$replaceWith": "$" }', kind: "operator", detail: "Replace document (alias)" },
  { label: "$set", insertText: '{ "$set": {} }', kind: "operator", detail: "Add fields (alias for $addFields)" },
  { label: "$unset", insertText: '{ "$unset": [""] }', kind: "operator", detail: "Remove fields" },
  { label: "$sortByCount", insertText: '{ "$sortByCount": "$" }', kind: "operator", detail: "Group and sort by count" },
  { label: "$graphLookup", insertText: '{ "$graphLookup": { "from": "", "startWith": "", "connectFromField": "", "connectToField": "", "as": "" } }', kind: "operator", detail: "Recursive search" },
  { label: "$geoNear", insertText: '{ "$geoNear": { "near": { "type": "Point", "coordinates": [0, 0] }, "distanceField": "dist" } }', kind: "operator", detail: "Geospatial proximity search" },
  { label: "$densify", insertText: '{ "$densify": { "field": "", "range": { "step": 1, "unit": "day" } } }', kind: "operator", detail: "Fill time-series gaps" },
  { label: "$fill", insertText: '{ "$fill": { "sortBy": { "": 1 }, "output": { "": { "method": "linear" } } } }', kind: "operator", detail: "Fill missing values" },
  { label: "$documents", insertText: '{ "$documents": [{}] }', kind: "operator", detail: "Literal documents" },
];

/** Aggregation expression operators. */
const EXPRESSION_OPERATORS: Suggestion[] = [
  // Accumulators
  { label: "$sum", insertText: '{ "$sum": "$" }', kind: "operator", detail: "Sum values" },
  { label: "$avg", insertText: '{ "$avg": "$" }', kind: "operator", detail: "Average" },
  { label: "$min", insertText: '{ "$min": "$" }', kind: "operator", detail: "Minimum" },
  { label: "$max", insertText: '{ "$max": "$" }', kind: "operator", detail: "Maximum" },
  { label: "$count", insertText: '{ "$count": {} }', kind: "operator", detail: "Count" },
  { label: "$first", insertText: '{ "$first": "$" }', kind: "operator", detail: "First value" },
  { label: "$last", insertText: '{ "$last": "$" }', kind: "operator", detail: "Last value" },
  { label: "$push", insertText: '{ "$push": "$" }', kind: "operator", detail: "Push to array" },
  { label: "$addToSet", insertText: '{ "$addToSet": "$" }', kind: "operator", detail: "Add unique" },
  // Conditional
  { label: "$ifNull", insertText: '{ "$ifNull": ["$", ""] }', kind: "operator", detail: "Replace null" },
  { label: "$cond", insertText: '{ "$cond": { "if": {}, "then": {}, "else": {} } }', kind: "operator", detail: "Conditional" },
  { label: "$switch", insertText: '{ "$switch": { "branches": [{ "case": {}, "then": {} }], "default": {} } }', kind: "operator", detail: "Switch statement" },
  // String
  { label: "$concat", insertText: '{ "$concat": ["$", ""] }', kind: "operator", detail: "Concatenate strings" },
  { label: "$split", insertText: '{ "$split": ["$", ","] }', kind: "operator", detail: "Split string" },
  { label: "$toLower", insertText: '{ "$toLower": "$" }', kind: "operator", detail: "Lowercase" },
  { label: "$toUpper", insertText: '{ "$toUpper": "$" }', kind: "operator", detail: "Uppercase" },
  { label: "$trim", insertText: '{ "$trim": { "input": "$" } }', kind: "operator", detail: "Trim whitespace" },
  { label: "$indexOfBytes", insertText: '{ "$indexOfBytes": ["$", ""] }', kind: "operator", detail: "Byte index of substring" },
  { label: "$indexOfCP", insertText: '{ "$indexOfCP": ["$", ""] }', kind: "operator", detail: "Code-point index of substring" },
  { label: "$strLenBytes", insertText: '{ "$strLenBytes": "$" }', kind: "operator", detail: "Byte length" },
  { label: "$strLenCP", insertText: '{ "$strLenCP": "$" }', kind: "operator", detail: "Code-point length" },
  { label: "$substrBytes", insertText: '{ "$substrBytes": ["$", 0, 10] }', kind: "operator", detail: "Byte substring" },
  { label: "$substrCP", insertText: '{ "$substrCP": ["$", 0, 10] }', kind: "operator", detail: "Code-point substring" },
  { label: "$replaceOne", insertText: '{ "$replaceOne": { "input": "", "find": "", "replacement": "" } }', kind: "operator", detail: "Replace first occurrence" },
  { label: "$replaceAll", insertText: '{ "$replaceAll": { "input": "", "find": "", "replacement": "" } }', kind: "operator", detail: "Replace all occurrences" },
  // Array
  { label: "$arrayElemAt", insertText: '{ "$arrayElemAt": ["$", 0] }', kind: "operator", detail: "Element at index" },
  { label: "$arrayToObject", insertText: '{ "$arrayToObject": "$" }', kind: "operator", detail: "Array to object" },
  { label: "$concatArrays", insertText: '{ "$concatArrays": ["$"] }', kind: "operator", detail: "Concatenate arrays" },
  { label: "$filter", insertText: '{ "$filter": { "input": "$", "as": "item", "cond": {} } }', kind: "operator", detail: "Filter array" },
  { label: "$indexOfArray", insertText: '{ "$indexOfArray": ["$", {}] }', kind: "operator", detail: "Index in array" },
  { label: "$isArray", insertText: '{ "$isArray": "$" }', kind: "operator", detail: "Is array" },
  { label: "$map", insertText: '{ "$map": { "input": "$", "as": "item", "in": {} } }', kind: "operator", detail: "Map array" },
  { label: "$objectToArray", insertText: '{ "$objectToArray": "$" }', kind: "operator", detail: "Object to array" },
  { label: "$range", insertText: '{ "$range": [0, 10] }', kind: "operator", detail: "Range of integers" },
  { label: "$reduce", insertText: '{ "$reduce": { "input": "$", "initialValue": {}, "in": {} } }', kind: "operator", detail: "Reduce array" },
  { label: "$reverseArray", insertText: '{ "$reverseArray": "$" }', kind: "operator", detail: "Reverse array" },
  { label: "$size", insertText: '{ "$size": "$" }', kind: "operator", detail: "Array size" },
  { label: "$slice", insertText: '{ "$slice": ["$", 10] }', kind: "operator", detail: "Slice array" },
  { label: "$zip", insertText: '{ "$zip": { "inputs": ["$"] } }', kind: "operator", detail: "Zip arrays" },
  { label: "$mergeObjects", insertText: '{ "$mergeObjects": ["$"] }', kind: "operator", detail: "Merge objects" },
  // Date
  { label: "$dateToString", insertText: '{ "$dateToString": { "format": "%Y-%m-%d", "date": "$" } }', kind: "operator", detail: "Format date" },
  { label: "$dateFromParts", insertText: '{ "$dateFromParts": { "year": 2024 } }', kind: "operator", detail: "Build date from parts" },
  { label: "$dateToParts", insertText: '{ "$dateToParts": { "date": "$" } }', kind: "operator", detail: "Decompose date to parts" },
  { label: "$dayOfMonth", insertText: '{ "$dayOfMonth": "$" }', kind: "operator", detail: "Day of month (1-31)" },
  { label: "$dayOfWeek", insertText: '{ "$dayOfWeek": "$" }', kind: "operator", detail: "Day of week (1-7)" },
  { label: "$dayOfYear", insertText: '{ "$dayOfYear": "$" }', kind: "operator", detail: "Day of year (1-366)" },
  { label: "$hour", insertText: '{ "$hour": "$" }', kind: "operator", detail: "Hour (0-23)" },
  { label: "$minute", insertText: '{ "$minute": "$" }', kind: "operator", detail: "Minute (0-59)" },
  { label: "$month", insertText: '{ "$month": "$" }', kind: "operator", detail: "Month (1-12)" },
  { label: "$second", insertText: '{ "$second": "$" }', kind: "operator", detail: "Second (0-59)" },
  { label: "$year", insertText: '{ "$year": "$" }', kind: "operator", detail: "Year" },
  { label: "$week", insertText: '{ "$week": "$" }', kind: "operator", detail: "Week (0-53)" },
  { label: "$isoWeek", insertText: '{ "$isoWeek": "$" }', kind: "operator", detail: "ISO week (1-53)" },
  { label: "$isoWeekYear", insertText: '{ "$isoWeekYear": "$" }', kind: "operator", detail: "ISO week-year" },
  { label: "$millisecond", insertText: '{ "$millisecond": "$" }', kind: "operator", detail: "Milliseconds (0-999)" },
  { label: "$dateAdd", insertText: '{ "$dateAdd": { "startDate": "$", "unit": "day", "amount": 1 } }', kind: "operator", detail: "Add to date" },
  { label: "$dateDiff", insertText: '{ "$dateDiff": { "startDate": "$", "endDate": "$", "unit": "day" } }', kind: "operator", detail: "Difference between dates" },
  { label: "$dateSubtract", insertText: '{ "$dateSubtract": { "startDate": "$", "unit": "day", "amount": 1 } }', kind: "operator", detail: "Subtract from date" },
  { label: "$dateTrunc", insertText: '{ "$dateTrunc": { "date": "$", "unit": "day" } }', kind: "operator", detail: "Truncate date" },
  // Math
  { label: "$abs", insertText: '{ "$abs": "$" }', kind: "operator", detail: "Absolute value" },
  { label: "$add", insertText: '{ "$add": ["$", 1] }', kind: "operator", detail: "Add numbers" },
  { label: "$ceil", insertText: '{ "$ceil": "$" }', kind: "operator", detail: "Ceiling" },
  { label: "$divide", insertText: '{ "$divide": ["$", 2] }', kind: "operator", detail: "Divide" },
  { label: "$exp", insertText: '{ "$exp": "$" }', kind: "operator", detail: "Exponent (e^x)" },
  { label: "$floor", insertText: '{ "$floor": "$" }', kind: "operator", detail: "Floor" },
  { label: "$ln", insertText: '{ "$ln": "$" }', kind: "operator", detail: "Natural logarithm" },
  { label: "$log", insertText: '{ "$log": ["$", 10] }', kind: "operator", detail: "Logarithm" },
  { label: "$log10", insertText: '{ "$log10": "$" }', kind: "operator", detail: "Base-10 logarithm" },
  { label: "$multiply", insertText: '{ "$multiply": ["$", 2] }', kind: "operator", detail: "Multiply" },
  { label: "$pow", insertText: '{ "$pow": ["$", 2] }', kind: "operator", detail: "Power" },
  { label: "$round", insertText: '{ "$round": ["$", 2] }', kind: "operator", detail: "Round to precision" },
  { label: "$sqrt", insertText: '{ "$sqrt": "$" }', kind: "operator", detail: "Square root" },
  { label: "$subtract", insertText: '{ "$subtract": ["$", 1] }', kind: "operator", detail: "Subtract" },
  { label: "$trunc", insertText: '{ "$trunc": "$" }', kind: "operator", detail: "Truncate to integer" },
  // Comparison (expression forms)
  { label: "$cmp", insertText: '{ "$cmp": ["$", ""] }', kind: "operator", detail: "Compare (-1, 0, 1)" },
  { label: "$eq", insertText: '{ "$eq": ["$", ""] }', kind: "operator", detail: "Expression equal" },
  { label: "$gt", insertText: '{ "$gt": ["$", ""] }', kind: "operator", detail: "Expression greater than" },
  { label: "$gte", insertText: '{ "$gte": ["$", ""] }', kind: "operator", detail: "Expression greater or equal" },
  { label: "$lt", insertText: '{ "$lt": ["$", ""] }', kind: "operator", detail: "Expression less than" },
  { label: "$lte", insertText: '{ "$lte": ["$", ""] }', kind: "operator", detail: "Expression less or equal" },
  { label: "$ne", insertText: '{ "$ne": ["$", ""] }', kind: "operator", detail: "Expression not equal" },
  // Boolean
  { label: "$and", insertText: '{ "$and": [{}] }', kind: "operator", detail: "Expression AND" },
  { label: "$or", insertText: '{ "$or": [{}] }', kind: "operator", detail: "Expression OR" },
  { label: "$not", insertText: '{ "$not": {} }', kind: "operator", detail: "Expression NOT" },
  // Type / conversion
  { label: "$toString", insertText: '{ "$toString": "$" }', kind: "operator", detail: "Convert to string" },
  { label: "$toDate", insertText: '{ "$toDate": "$" }', kind: "operator", detail: "Convert to date" },
  { label: "$toInt", insertText: '{ "$toInt": "$" }', kind: "operator", detail: "Convert to int" },
  { label: "$toDouble", insertText: '{ "$toDouble": "$" }', kind: "operator", detail: "Convert to double" },
  { label: "$toBool", insertText: '{ "$toBool": "$" }', kind: "operator", detail: "Convert to bool" },
  { label: "$toLong", insertText: '{ "$toLong": "$" }', kind: "operator", detail: "Convert to long" },
  { label: "$toObjectId", insertText: '{ "$toObjectId": "$" }', kind: "operator", detail: "Convert to ObjectId" },
  { label: "$convert", insertText: '{ "$convert": { "input": "$", "to": "string" } }', kind: "operator", detail: "Convert type" },
  { label: "$type", insertText: '{ "$type": "$" }', kind: "operator", detail: "BSON type of value" },
  // Set
  { label: "$setDifference", insertText: '{ "$setDifference": ["$", []] }', kind: "operator", detail: "Set difference" },
  { label: "$setEquals", insertText: '{ "$setEquals": ["$", []] }', kind: "operator", detail: "Set equality" },
  { label: "$setIntersection", insertText: '{ "$setIntersection": ["$", []] }', kind: "operator", detail: "Set intersection" },
  { label: "$setIsSubset", insertText: '{ "$setIsSubset": ["$", []] }', kind: "operator", detail: "Subset check" },
  { label: "$setUnion", insertText: '{ "$setUnion": ["$", []] }', kind: "operator", detail: "Set union" },
  // Misc
  { label: "$let", insertText: '{ "$let": { "vars": {}, "in": {} } }', kind: "operator", detail: "Local variables" },
  { label: "$literal", insertText: '{ "$literal": "" }', kind: "operator", detail: "Literal value" },
  { label: "$rand", insertText: '{ "$rand": {} }', kind: "operator", detail: "Random 0-1" },
  { label: "$regexMatch", insertText: '{ "$regexMatch": { "input": "", "regex": "" } }', kind: "operator", detail: "Regex match" },
  { label: "$substr", insertText: '{ "$substr": ["$", 0, 10] }', kind: "operator", detail: "Substring (legacy)" },
];

/** SQL keywords for the SQL tab. */
const SQL_KEYWORDS: Suggestion[] = [
  "SELECT", "FROM", "WHERE", "ORDER BY", "LIMIT", "OFFSET",
  "GROUP BY", "HAVING", "JOIN", "INNER JOIN", "LEFT JOIN",
  "RIGHT JOIN", "FULL JOIN", "CROSS JOIN",
  "ON", "AND", "OR", "NOT", "IN", "EXISTS",
  "INSERT INTO", "VALUES", "UPDATE", "SET", "DELETE FROM",
  "COUNT(*)", "SUM", "AVG", "MIN", "MAX", "DISTINCT",
  "ASC", "DESC", "LIKE", "IS NULL", "IS NOT NULL",
  "BETWEEN", "CASE WHEN", "THEN", "ELSE", "END",
  "UNION", "UNION ALL",
  "WITH",
  "CAST",
  "CONCAT",
  "NOW()", "CURRENT_DATE", "CURRENT_TIMESTAMP",
  "TRUE", "FALSE", "NULL",
  "UPSERT", "AS",
].map((kw) => ({
  label: kw,
  insertText: `${kw} `,
  kind: "keyword" as const,
}));

/** BSON type aliases and codes for $type operator values. */
const BSON_TYPE_VALUES: Suggestion[] = [
  { label: "double", insertText: '"double"', kind: "value", detail: "BSON type 1" },
  { label: "string", insertText: '"string"', kind: "value", detail: "BSON type 2" },
  { label: "object", insertText: '"object"', kind: "value", detail: "BSON type 3" },
  { label: "array", insertText: '"array"', kind: "value", detail: "BSON type 4" },
  { label: "binData", insertText: '"binData"', kind: "value", detail: "BSON type 5" },
  { label: "undefined", insertText: '"undefined"', kind: "value", detail: "BSON type 6 (deprecated)" },
  { label: "objectId", insertText: '"objectId"', kind: "value", detail: "BSON type 7" },
  { label: "bool", insertText: '"bool"', kind: "value", detail: "BSON type 8" },
  { label: "date", insertText: '"date"', kind: "value", detail: "BSON type 9" },
  { label: "null", insertText: '"null"', kind: "value", detail: "BSON type 10" },
  { label: "regex", insertText: '"regex"', kind: "value", detail: "BSON type 11" },
  { label: "dbPointer", insertText: '"dbPointer"', kind: "value", detail: "BSON type 12 (deprecated)" },
  { label: "javascript", insertText: '"javascript"', kind: "value", detail: "BSON type 13" },
  { label: "symbol", insertText: '"symbol"', kind: "value", detail: "BSON type 14 (deprecated)" },
  { label: "javascriptWithScope", insertText: '"javascriptWithScope"', kind: "value", detail: "BSON type 15" },
  { label: "int", insertText: '"int"', kind: "value", detail: "BSON type 16" },
  { label: "timestamp", insertText: '"timestamp"', kind: "value", detail: "BSON type 17" },
  { label: "long", insertText: '"long"', kind: "value", detail: "BSON type 18" },
  { label: "decimal", insertText: '"decimal"', kind: "value", detail: "BSON type 19" },
  { label: "minKey", insertText: '"minKey"', kind: "value", detail: "BSON type -1" },
  { label: "maxKey", insertText: '"maxKey"', kind: "value", detail: "BSON type 127" },
];

/** Date tag shortcuts supported by the bson_json parser. */
const DATE_TAG_VALUES: Suggestion[] = [
  { label: "#today", insertText: '"#today"', kind: "value", detail: "Today at midnight" },
  { label: "#now", insertText: '"#now"', kind: "value", detail: "Current timestamp" },
  { label: "#yesterday", insertText: '"#yesterday"', kind: "value", detail: "Yesterday at midnight" },
  { label: "#tomorrow", insertText: '"#tomorrow"', kind: "value", detail: "Tomorrow at midnight" },
  { label: "#lastweek", insertText: '"#lastweek"', kind: "value", detail: "7 days ago" },
  { label: "#nextweek", insertText: '"#nextweek"', kind: "value", detail: "7 days from now" },
  { label: "#lastmonth", insertText: '"#lastmonth"', kind: "value", detail: "30 days ago" },
  { label: "#nextmonth", insertText: '"#nextmonth"', kind: "value", detail: "30 days from now" },
];

// ─── Context detection ───────────────────────────────────────────────

/** Tokenize a small window around the cursor to figure out context. */
function tokenizeAround(text: string, offset: number): { before: string; after: string; inString: boolean; stringChar: string } {
  let inString = false;
  let stringChar = "";
  let escaped = false;

  for (let i = 0; i < offset; i++) {
    const c = text[i];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (c === "\\") {
      escaped = true;
      continue;
    }
    if (inString) {
      if (c === stringChar) inString = false;
    } else {
      if (c === '"' || c === "'") {
        inString = true;
        stringChar = c;
      }
    }
  }

  // If we're inside a string, find the start of that string
  const before = text.slice(0, offset);
  const after = text.slice(offset);
  return { before, after, inString, stringChar };
}

/** Find the word fragment the user is currently typing. */
function getWordAtOffset(text: string, offset: number): { word: string; start: number } {
  let start = offset;
  while (start > 0) {
    const c = text[start - 1];
    if (/[\w$_.]/.test(c)) start--;
    else break;
  }
  return { word: text.slice(start, offset), start };
}

/**
 * Forward-parse the text up to `offset` to determine which object keys we
 * are currently nested inside. Uses a scope stack so unmatched braces and
 * nested objects are tracked accurately.
 */
function findParentKeys(text: string, offset: number): string[] {
  type Frame = { type: "obj" | "arr"; key: string | null };
  const stack: Frame[] = [];
  let i = 0;
  let inStr = false;
  let strChar = "";
  let esc = false;
  let lastStringValue: string | null = null;

  while (i < offset) {
    const c = text[i];
    if (esc) {
      esc = false;
      i++;
      continue;
    }
    if (c === "\\") {
      esc = true;
      i++;
      continue;
    }
    if (inStr) {
      if (c === strChar) inStr = false;
      i++;
      continue;
    }
    if (c === '"' || c === "'") {
      const start = i + 1;
      let j = start;
      while (j < offset) {
        if (text[j] === "\\") {
          j += 2;
          continue;
        }
        if (text[j] === c) break;
        j++;
      }
      lastStringValue = text.slice(start, j);
      // If the next non-whitespace after the closing quote is ':',
      // this string is a key in the current object. Only set the key
      // on the frame if it hasn't been set yet (the first key seen
      // after a '{' is the one that introduced the object scope).
      let k = j + 1;
      while (k < offset && /\s/.test(text[k])) k++;
      if (k < offset && text[k] === ":") {
        const top = stack[stack.length - 1];
        if (top && top.type === "obj" && top.key === null) {
          top.key = lastStringValue;
        }
      }
      inStr = true;
      strChar = c;
      i++;
      continue;
    }
    if (c === "{") {
      // The '{' may be the value of the last seen key.
      const key = lastStringValue !== null ? lastStringValue : null;
      stack.push({ type: "obj", key });
    } else if (c === "}") {
      stack.pop();
    } else if (c === "[") {
      stack.push({ type: "arr", key: null });
    } else if (c === "]") {
      stack.pop();
    } else if (c === ":") {
      // key already consumed via lastStringValue
    }
    i++;
  }
  return stack.filter((f) => f.type === "obj" && f.key !== null).map((f) => f.key!);
}

/** Heuristic: what context are we in? */
function inferJsonContext(
  text: string,
  offset: number,
): {
  isKey: boolean;
  parentKeys: string[];
  prefix: string;
  replaceStart: number;
} {
  const { before, inString } = tokenizeAround(text, offset);

  // If we're not in a string, try to determine if we're at a key position
  const trimmed = before.trimEnd();
  const lastChar = trimmed.slice(-1);
  const isKey = inString || lastChar === "{" || lastChar === ",";

  const { word, start } = getWordAtOffset(text, offset);
  const parentKeys = findParentKeys(text, offset);

  return { isKey, parentKeys, prefix: word, replaceStart: start };
}

// ─── Public API ──────────────────────────────────────────────────────

export type EditorContext = "filter" | "update" | "aggregate" | "insert" | "sql";

export function getSuggestions(
  text: string,
  offset: number,
  context: EditorContext,
  schema?: { topLevelFields: string[]; allPaths: string[]; childrenByPrefix: Map<string, string[]> },
): CompletionResult {
  const { isKey, parentKeys, prefix, replaceStart } = inferJsonContext(text, offset);
  const lowerPrefix = prefix.toLowerCase();

  const suggestions: Suggestion[] = [];

  if (context === "sql") {
    // Word-based SQL completion that also handles multi-word keywords
    // like "ORDER BY" by looking at the preceding word.
    const { word, start } = getWordAtOffset(text, offset);
    const before = text.slice(0, start).trimEnd();
    const prevWordMatch = before.match(/(\S+)$/);
    const prevWord = prevWordMatch ? prevWordMatch[1] : "";
    const combined = prevWord ? `${prevWord} ${word}` : word;
    const combinedUpper = combined.toUpperCase();
    const wordUpper = word.toUpperCase();

    let matches = SQL_KEYWORDS.filter((k) =>
      k.label.toUpperCase().startsWith(combinedUpper),
    );
    // Fallback: if no multi-word match, try single-word
    if (matches.length === 0) {
      matches = SQL_KEYWORDS.filter((k) =>
        k.label.toUpperCase().startsWith(wordUpper),
      );
    }
    // Determine replacement span: if combined matched, replace the previous word too
    const replaceStart = combinedUpper !== wordUpper && matches.some((m) =>
      m.label.toUpperCase().startsWith(combinedUpper),
    )
      ? before.length - prevWord.length
      : start;

    return {
      suggestions: matches.slice(0, 50),
      replaceStart,
      replaceEnd: offset,
    };
  }

  // ── Operator suggestions ──────────────────────────────────────
  if (prefix.startsWith("$") || (isKey && parentKeys.length === 0 && context !== "insert")) {
    let ops: Suggestion[] = [];
    if (context === "update") ops = UPDATE_OPERATORS;
    else if (context === "aggregate") {
      // Distinguish pipeline stages vs expressions
      if (parentKeys.length === 0) {
        ops = STAGE_OPERATORS;
      } else {
        ops = EXPRESSION_OPERATORS;
      }
    } else {
      ops = FILTER_OPERATORS;
    }
    const matches = ops.filter((o) => o.label.toLowerCase().startsWith(lowerPrefix));
    suggestions.push(...matches);
  }

  // ── Field suggestions ─────────────────────────────────────────
  if (schema && isKey && !prefix.startsWith("$")) {
    let candidates: string[] = [];
    if (parentKeys.length === 0 || context === "insert") {
      candidates = schema.topLevelFields;
    } else {
      const immediateParent = parentKeys[parentKeys.length - 1];
      const children = schema.childrenByPrefix.get(immediateParent);
      if (children && children.length > 0) {
        candidates = children.map((p) => p.slice(immediateParent.length + 1));
      } else {
        candidates = schema.topLevelFields;
      }
    }
    const matches = candidates.filter((f) => f.toLowerCase().startsWith(lowerPrefix));
    suggestions.push(
      ...matches.map((f) => ({
        label: f,
        insertText: `"${f}": `,
        kind: "field" as const,
        detail: "Field",
      })),
    );
  }

  // ── Value suggestions ───────────────────────────────────────
  // Suggest BSON type names when the cursor is inside a $type value
  if (parentKeys[parentKeys.length - 1] === "$type") {
    const matches = BSON_TYPE_VALUES.filter((v) =>
      v.label.toLowerCase().startsWith(lowerPrefix),
    );
    suggestions.push(...matches);
  }

  // Suggest date tags when inside a string value in filter/update contexts
  const { inString } = tokenizeAround(text, offset);
  if (inString && (context === "filter" || context === "update")) {
    const tagPrefix = prefix.startsWith("#") ? prefix : `#${prefix}`;
    const matches = DATE_TAG_VALUES.filter((v) =>
      v.label.toLowerCase().startsWith(tagPrefix.toLowerCase()),
    );
    suggestions.push(...matches);
  }

  // ── Common snippets ───────────────────────────────────────────
  if (context === "filter" && prefix === "" && parentKeys.length === 0) {
    suggestions.push({
      label: "ObjectId filter",
      insertText: '{ "_id": ObjectId("") }',
      kind: "snippet",
      detail: "Find by ObjectId",
    });
  }
  if (context === "update" && prefix === "" && parentKeys.length === 0) {
    suggestions.push({
      label: "Set fields",
      insertText: '{ "$set": { } }',
      kind: "snippet",
      detail: "Update with $set",
    });
  }

  // Rank: exact prefix matches first, then alphabetically
  const ranked = suggestions.sort((a, b) => {
    const aExact = a.label.toLowerCase() === lowerPrefix ? 0 : 1;
    const bExact = b.label.toLowerCase() === lowerPrefix ? 0 : 1;
    if (aExact !== bExact) return aExact - bExact;
    return a.label.localeCompare(b.label);
  });

  return {
    suggestions: ranked.slice(0, 50),
    replaceStart,
    replaceEnd: offset,
  };
}
