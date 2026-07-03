// Typed IPC command wrappers for the frontend.
// Every command returns `Promise<T>` and the actual type matches the
// matching `#[tauri::command]` in src-tauri/src/commands/. Any error
// the Rust side produces is serialized as `{ kind, message }`.

import { invoke } from "@tauri-apps/api/core";

/** Format a Tauri/Rust error for display. Errors are serialized as `{ kind, message }`. */
export function formatError(err: unknown): string {
  const raw = extractRawMessage(err);
  const friendly = humanizeError(raw);
  if (friendly && friendly !== raw) {
    console.error("[nosqlbuddy] error detail:", raw);
  }
  return friendly;
}

/** Extract the raw error string from various error shapes. */
function extractRawMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  if (err && typeof err === "object") {
    const obj = err as Record<string, unknown>;
    if (typeof obj.message === "string") return obj.message;
    if (typeof obj.error === "string") return obj.error;
    if (typeof obj.kind === "string" && typeof obj.message === "string") {
      return `${obj.kind}: ${obj.message}`;
    }
    return JSON.stringify(err);
  }
  return String(err);
}

/** Convert a raw driver/internal error into a short, human-friendly message. */
function humanizeError(raw: string): string {
  // MongoDB driver: server selection timeout (host unreachable / wrong port / firewall)
  if (raw.includes("Server selection timeout") || raw.includes("No available servers")) {
    const addrMatch = raw.match(/Address:\s*([\d.]+:\d+)/);
    const addr = addrMatch ? addrMatch[1] : "the server";
    if (raw.includes("Connection refused")) {
      return `Could not connect to ${addr}. The server is not reachable (connection refused). Check that MongoDB is running and the port is correct.`;
    }
    if (raw.includes("timed out")) {
      return `Connection to ${addr} timed out. The server may be behind a firewall or unreachable from your network.`;
    }
    return `Could not connect to ${addr}. Check that the host and port are correct, and that MongoDB is running.`;
  }

  // MongoDB driver: authentication failed
  if (raw.includes("Authentication failed") || raw.includes("auth fail")) {
    return "Authentication failed. Check your username, password, and auth mechanism.";
  }

  // MongoDB driver: not writable primary
  if (raw.includes("NotWritablePrimary") || raw.includes("not master")) {
    return "The connected server is not the primary. Use a replica set URI or wait for an election to complete.";
  }

  // MongoDB driver: bad auth mechanism
  if (raw.includes("Unsupported OP_MSG") || raw.includes("SASL")) {
    return "The server rejected the authentication mechanism. Try a different auth method (e.g. SCRAM-SHA-256).";
  }

  // DNS / SRV resolution
  if (raw.includes("Failed to lookup") || raw.includes("DNS") || raw.includes("nodename nor servname provided")) {
    return "Could not resolve the hostname. Check the connection URI for typos.";
  }

  // TLS / SSL
  if (raw.includes("TLS") || raw.includes("SSL") || raw.includes("certificate")) {
    return "TLS/SSL error. If using a self-signed certificate, you may need to allow invalid certificates in the URI.";
  }

  // Network: connection reset
  if (raw.includes("Connection reset") || raw.includes("broken pipe")) {
    return "The connection was unexpectedly closed by the server. Try again.";
  }

  // Tauri command invocation error
  if (raw.startsWith("Internal: ")) {
    return raw.replace("Internal: ", "").trim();
  }

  // Validation errors from the Rust side
  if (raw.startsWith("Validation: ")) {
    return raw.replace("Validation: ", "").trim();
  }

  // Fallback: return as-is if short, truncate if very long
  if (raw.length > 300) {
    return raw.slice(0, 300) + "...";
  }
  return raw;
}

/** Profile summary as shown in the connection tree. */
export interface ProfileSummary {
  id: string;
  name: string;
  maskedUri: string;
  authMechanism:
    | "none"
    | "scram-sha-1"
    | "scram-sha-256"
    | "x509"
    | "ldap"
    | "kerberos"
    | "aws-iam";
  hasSecret: boolean;
  group: string | null;
  color: string | null;
  notes: string | null;
  sshTunnel: SshTunnelConfig | null;
  socks5: Socks5Config | null;
  tls: TlsConfig | null;
}

export interface SshTunnelConfig {
  host: string;
  port: number;
  user: string;
  privateKeyPath: string | null;
  password: string | null;
}

export interface Socks5Config {
  host: string;
  port: number;
  user: string | null;
  password: string | null;
}

export interface TlsConfig {
  enabled: boolean | null;
  certKeyFile: string | null;
  caFile: string | null;
  allowInvalidCertificates: boolean | null;
}

export interface SaveProfileRequest {
  id?: string;
  name: string;
  uri: string;
  authMechanism: ProfileSummary["authMechanism"];
  secret?: string;
  group?: string | null;
  color?: string | null;
  notes?: string | null;
  sshTunnel?: SshTunnelConfig | null;
  socks5?: Socks5Config | null;
  tls?: TlsConfig | null;
}

export interface TestResult {
  ok: boolean;
  message: string;
  latencyMs: number | null;
}

export interface ServerInfo {
  version: string | null;
  host: string | null;
  isMaster: boolean | null;
  topology: string | null;
}

export interface DatabaseSummary {
  name: string;
  sizeOnDisk: number | null;
  collectionsCount: number | null;
  documentCount: number | null;
  indexCount: number | null;
  indexSizeBytes: number | null;
  storageSizeBytes: number | null;
}

export type CollectionKind =
  | "collection"
  | "view"
  | "time-series"
  | "sharded"
  | "bucketed";

export interface CollectionSummary {
  name: string;
  type: CollectionKind;
  documentCount: number | null;
  sizeBytes: number | null;
  storageSizeBytes: number | null;
}

export interface ConnectionHandle {
  connectionId: string;
  profileId: string;
  name: string;
  serverInfo: ServerInfo | null;
  databases: DatabaseSummary[];
}

export interface ConnectionDescriptor {
  connectionId: string;
  profileId: string;
  name: string;
  openedAt: string;
}

export interface DocumentPage {
  documents: Array<Record<string, unknown>>;
  limit: number;
  skip: number;
  hasMore: boolean;
  executionMs: number | null;
  totalCount: number | null;
  /**
   * `true` when `totalCount` came from `estimatedDocumentCount` (collection
   * metadata, ~constant time, approximate). Render such counts with a
   * leading "≈". `false` (or undefined) means an exact `countDocuments`
   * against the filter.
   */
  totalCountApprox?: boolean;
}

/**
 * How the backend computes `totalCount` for a paged find.
 * - `estimated` (default): collection metadata via `estimatedDocumentCount`,
 *   ~constant time. Rendered with "≈" in the UI.
 * - `exact`: real `countDocuments` against the filter. O(scan); opt in only
 *   for filtered queries where an estimate would mislead.
 * - `none`: skip the count entirely for the lowest-latency first paint.
 */
export type CountMode = "estimated" | "exact" | "none";

export interface CollectionStats {
  name: string;
  documentCount: number;
  sizeBytes: number;
  storageSizeBytes: number;
  indexCount: number;
  totalIndexSizeBytes: number;
  avgObjSizeBytes: number;
}

export interface CollationDto {
  locale: string;
  strength?: number | null;
  caseLevel?: boolean | null;
  caseFirst?: string | null;
  numericOrdering?: boolean | null;
  alternate?: string | null;
  maxVariable?: string | null;
  normalization?: boolean | null;
  backwards?: boolean | null;
}

export interface IndexInfo {
  name: string;
  key: Record<string, unknown>;
  unique: boolean;
  sparse: boolean;
  hidden: boolean;
  ttlSeconds: number | null;
  partialFilterExpression: Record<string, unknown> | null;
  collation: CollationDto | null;
  wildcardProjection: Record<string, unknown> | null;
  isText: boolean;
  isGeo: boolean;
  isId: boolean;
}

export interface IndexStats {
  name: string;
  ops: number;
  sinceMs: number | null;
  accesses: number | null;
  sizeBytes: number | null;
  building: boolean | null;
  metadata: Record<string, unknown> | null;
}

export interface ExplainResult {
  queryPlannerWinningPlan: Record<string, unknown>;
  executionStats: Record<string, unknown> | null;
  raw: Record<string, unknown>;
}

export interface SchemaValueCount {
  value: string;
  count: number;
}

export interface SchemaBucket {
  lo: number;
  hi: number;
  count: number;
}

export interface SchemaNumericStats {
  min: number;
  max: number;
  mean: number;
  buckets: SchemaBucket[];
}

export interface SchemaDateBucket {
  loMs: number;
  hiMs: number;
  count: number;
}

export interface SchemaDateStats {
  minMs: number;
  maxMs: number;
  buckets: SchemaDateBucket[];
}

export interface SchemaField {
  name: string;
  types: Record<string, number>;
  nullRatio: number;
  missingCount: number;
  topValues: SchemaValueCount[] | null;
  numericStats: SchemaNumericStats | null;
  dateStats: SchemaDateStats | null;
}

export interface SchemaReport {
  sampledDocuments: number;
  fields: SchemaField[];
}

// ─── Shape (recursive document-shape inference) ──────────────────────

export type ShapeType =
  | "objectId"
  | "string"
  | "int"
  | "long"
  | "double"
  | "decimal"
  | "bool"
  | "date"
  | "object"
  | "array"
  | "null"
  | "binary"
  | "timestamp"
  | "other";

export interface ShapeNode {
  path: string;
  name: string;
  types: Record<string, number>;
  presence: number;
  nullRatio: number;
  cardinality?: number | null;
  children: ShapeNode[];
  arrayItem?: ShapeNode | null;
  topValues: SchemaValueCount[] | null;
  numericStats: SchemaNumericStats | null;
  dateStats: SchemaDateStats | null;
}

export interface CollectionShape {
  database: string;
  collection: string;
  kind: CollectionKind;
  documentCount: number | null;
  sampledDocuments: number;
  root: ShapeNode;
  maxDepth: number;
  warnings: string[];
  indexes: IndexInfo[];
}

// ─── Data Model / Relationships ─────────────────────────────────────

export type RelationshipKind = "one-to-one" | "one-to-many" | "many-to-one" | "many-to-many";

export type SignalKind = "objectIdMatch" | "namingConvention" | "lookup" | "index" | "appSchema";

export interface RelationshipSignal {
  kind: SignalKind;
  detail: string;
  weight: number;
}

export interface RelationshipEdge {
  id: string;
  fromCollection: string;
  toCollection: string;
  fromField: string;
  toField: string;
  kind: RelationshipKind;
  confidence: number;
  signals: RelationshipSignal[];
  viaCollection?: string | null;
  confirmed?: boolean;
  hidden?: boolean;
}

/** A `$lookup`-derived relationship signal parsed from query history. */
export interface LookupSignal {
  fromCollection: string;
  toCollection: string;
  localField: string;
  foreignField: string;
  count: number;
}

export interface DataModelGraph {
  database: string;
  nodes: CollectionShape[];
  edges: RelationshipEdge[];
  generatedAt: string;
  sampleSize: number;
  confidenceThreshold: number;
  warnings: string[];
}

export interface ScanScopeRequest {
  connectionId: string;
  database: string;
  collections: string[];
  sampleSize: number;
  signals: {
    objectIdMatch: boolean;
    naming: boolean;
    lookup: boolean;
    index: boolean;
    appSchema: boolean;
  };
  confidenceThreshold: number;
  appSchemaPath?: string | null;
  /** `$lookup` signals parsed from the frontend query history. */
  lookupSignals?: LookupSignal[];
}

export type SqlOperation =
  | { kind: "find" }
  | { kind: "aggregate" }
  | { kind: "update"; filter: Record<string, unknown>; update: Record<string, unknown>; multi: boolean; upsert: boolean }
  | { kind: "insert"; documents: unknown[] }
  | { kind: "delete"; filter: Record<string, unknown>; multi: boolean }
  | { kind: "replace"; filter: Record<string, unknown>; replacement: Record<string, unknown>; upsert: boolean };

export interface SqlTranslation {
  database: string;
  collection: string;
  operation: SqlOperation;
  pipeline: unknown[];
  find: Record<string, unknown> | null;
  warnings: string[];
  code: Record<string, string>;
}

export type SqlLanguage =
  | "node-js"
  | "python"
  | "java"
  | "c-sharp"
  | "ruby"
  | "shell";

// ─── Import / Export ────────────────────────────────────────────────

export type ExportSourceMode = "find" | "aggregate" | "documents";
export type ExportFormat = "json" | "csv" | "bson";
export type ExportDestinationKind = "file" | "clipboard";
export type JsonShape = "array" | "ndjson";
export type CompressionFormat = "none" | "gzip" | "zstd";
export type CsvArrayMode = "jsonString" | "flatten";

export interface ExportSourceDto {
  mode: ExportSourceMode;
  filterJson?: string | null;
  projectionJson?: string | null;
  sortJson?: string | null;
  pipelineJson?: string | null;
  documentsJson?: string | null;
}

export interface ExportDestinationDto {
  kind: ExportDestinationKind;
  path?: string | null;
}

export interface ExportOptions {
  jsonShape: JsonShape;
  canonical: boolean;
  csvDelimiter?: string | null;
  csvHeaders: boolean;
  csvColumns?: string[] | null;
  compression: CompressionFormat;
  csvArrayMode?: CsvArrayMode | null;
  /**
   * Optional field-mapping table applied as a transform before the sink. When
   * present and non-empty, the mapping is the complete output schema:
   * undeclared fields are dropped. For CSV, the sink columns are derived from
   * the non-skipped `target` names in declared order, overriding `csvColumns`
   * and the schema sample.
   */
  fieldMapping?: FieldMappingEntry[] | null;
}

export interface ExportRequest {
  connectionId: string;
  database: string;
  collection: string;
  jobId: string;
  source: ExportSourceDto;
  format: ExportFormat;
  destination: ExportDestinationDto;
  options: ExportOptions;
}

export interface ExportResult {
  jobId: string;
  processed: number;
  errors: number;
  cancelled: boolean;
  path: string | null;
  clipboardText: string | null;
}

export type ImportFormat = "json" | "csv" | "bson";
export type ImportSourceKind = "file" | "clipboard";
export type JsonImportShape = "object" | "array" | "ndjson";

/**
 * One row in the field-mapping table. `source` is a dotted path into the
 * incoming document (e.g. `address.city`); `target` is the output field name.
 * `skip` drops the field; `typeOverride` coerces the BSON value to the declared
 * type. Matches `FieldMappingEntry` / `TypeOverride` in
 * `src-tauri/src/mongo/import_export/mapping.rs`.
 */
export type TypeOverride =
  | "string"
  | "int32"
  | "int64"
  | "double"
  | "boolean"
  | "date"
  | "objectId";

export interface FieldMappingEntry {
  source: string;
  target: string;
  skip: boolean;
  typeOverride?: TypeOverride | null;
}

/** The set of tokens supported in export destination path placeholders.
 * Matches `placeholders.rs`. Unknown `${...}` tokens are left intact. */
export const PLACEHOLDER_TOKENS = [
  "${date}",
  "${time}",
  "${db}",
  "${collection}",
  "${profile}",
] as const;
export type PlaceholderToken = (typeof PLACEHOLDER_TOKENS)[number];

export interface ImportSourceDto {
  kind: ImportSourceKind;
  path?: string | null;
  clipboardText?: string | null;
}

export interface ImportOptions {
  jsonShape: JsonImportShape;
  csvDelimiter?: string | null;
  csvHeaders: boolean;
  batchSize?: number | null;
  previewRows?: number | null;
  /** Optional field-mapping table applied before the collection sink. */
  fieldMapping?: FieldMappingEntry[] | null;
}

export interface ImportRequest {
  connectionId: string;
  database: string;
  collection: string;
  jobId: string;
  source: ImportSourceDto;
  format: ImportFormat;
  options: ImportOptions;
}

export interface ImportRowError {
  row: number | null;
  message: string;
}

export interface FieldInference {
  name: string;
  bsonType: string;
  nullable: boolean;
  samples: string[];
}

export interface PreviewImportResult {
  rows: unknown[];
  fields: FieldInference[];
  errors: ImportRowError[];
}

export interface ImportResult {
  jobId: string;
  processed: number;
  inserted: number;
  errors: number;
  cancelled: boolean;
  rowErrors: ImportRowError[];
}

export interface CopyTargetDto {
  database: string;
  collection: string;
}

export interface CopyRequest {
  connectionId: string;
  database: string;
  collection: string;
  jobId: string;
  source: ExportSourceDto;
  target: CopyTargetDto;
  batchSize?: number | null;
}

export interface CopyResult {
  jobId: string;
  processed: number;
  inserted: number;
  errors: number;
  cancelled: boolean;
}

// ─── Jobs ─────────────────────────────────────────────────────────

export type JobKind = "dump" | "restore" | "export" | "import";
export type JobStatus = "queued" | "running" | "done" | "failed" | "cancelled";

export interface ScheduleConfig {
  cron: string;
  enabled: boolean;
  retentionCount: number | null;
  nextRunAt: string | null;
}

export interface JobMeta {
  jobId: string;
  kind: JobKind;
  status: JobStatus;
  connectionId: string;
  profileId: string;
  database: string;
  collections: string[];
  createdAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  outputPath: string | null;
  sourcePath: string | null;
  schedule: ScheduleConfig | null;
  parentJobId: string | null;
  processed: number;
  total: number | null;
  errors: number;
  message: string;
}

export interface JobFilterRequest {
  connectionId?: string | null;
  profileId?: string | null;
  database?: string | null;
  kind?: string | null;
  status?: string | null;
  limit?: number | null;
}

export interface JobListResponse {
  jobs: JobMeta[];
}

export interface JobLogEntry {
  timestamp: string;
  level: "info" | "warn" | "error";
  message: string;
}

export interface JobDetailResponse {
  jobId: string;
  kind: JobKind;
  status: JobStatus;
  connectionId: string;
  profileId: string;
  database: string;
  collections: string[];
  createdAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  outputPath: string | null;
  sourcePath: string | null;
  schedule: ScheduleConfig | null;
  parentJobId: string | null;
  processed: number;
  total: number | null;
  errors: number;
  message: string;
  logs: JobLogEntry[];
}

// ─── Dump / Restore ───────────────────────────────────────────────

export type ConflictStrategy = "drop" | "skip" | "upsert";

export interface CollectionMapping {
  source: string;
  target: string;
  enabled: boolean;
}

export interface DumpRequest {
  connectionId: string;
  database: string;
  collections: string[];
  destinationDir: string;
  pathTemplate: string;
  format: "bson" | "json";
  compression: CompressionFormat;
  jobId: string;
}

export interface DumpResult {
  jobId: string;
  processed: number;
  errors: number;
  cancelled: boolean;
  files: string[];
}

export interface RestoreRequest {
  connectionId: string;
  sourceDir: string;
  targetDatabase: string;
  createDatabase: boolean;
  collectionMap: CollectionMapping[];
  conflictStrategy: ConflictStrategy;
  jobId: string;
}

export interface RestoreResult {
  jobId: string;
  processed: number;
  inserted: number;
  errors: number;
  cancelled: boolean;
}

export interface ArchivePreviewEntry {
  sourceName: string;
  targetName: string;
  approximateCount: number;
  sizeBytes: number;
}

export interface AppInfo {
  platform: string;
  arch: string;
  tauriVersion: string;
  appName: string;
  appVersion: string;
}

// ─── Safe Change Mode ────────────────────────────────────────────────

/** Configuration for Safe Change Mode (stored in app settings). */
export interface SafeChangeSettings {
  enabled: boolean;
  requireTypedConfirmationThreshold: number;
  alwaysPreviewOnProduction: boolean;
}

export interface AppSettings {
  theme: "system" | "light" | "dark";
  lastConnectionId: string | null;
  safeChange?: SafeChangeSettings;
}

export const DEFAULT_SAFE_CHANGE_SETTINGS: SafeChangeSettings = {
  enabled: true,
  requireTypedConfirmationThreshold: 60,
  alwaysPreviewOnProduction: true,
};

export type SafeChangeOperationKind =
  | "updateOne"
  | "updateMany"
  | "deleteOne"
  | "deleteMany"
  | "replaceOne";

export interface SafeChangePreviewRequest {
  connectionId: string;
  database: string;
  collection: string;
  kind: SafeChangeOperationKind;
  filterJson: string;
  updateJson?: string | null;
  replacementJson?: string | null;
  sampleLimit?: number | null;
}

export type SafeChangeRollbackLevel = "metadataOnly" | "sampleBased" | "full";
export type SafeChangeType = "added" | "modified" | "removed";

export interface SafeChangeFieldChange {
  field: string;
  oldValue: unknown;
  newValue: unknown;
  changeType: SafeChangeType;
}

export interface SafeChangeDocumentDiff {
  documentIndex: number;
  fieldChanges: SafeChangeFieldChange[];
}

export interface SafeChangeIndexInfo {
  indexUsed: boolean;
  stage: string;
}

export interface SafeChangePreview {
  kind: SafeChangeOperationKind;
  matchedCount: number;
  sampleBefore: string[];
  sampleAfter: string[];
  diffs: SafeChangeDocumentDiff[];
  riskScore: number;
  riskReasons: string[];
  warnings: string[];
  rollbackScript: string;
  rollbackLevel: SafeChangeRollbackLevel;
  requiresTypedConfirmation: boolean;
  confirmationText: string;
  isProduction: boolean;
  indexInfo: SafeChangeIndexInfo;
}

/** Rollback metadata carried on write IPC calls so the timeline entry can
 *  store the Safe Change preview data atomically with the operation. */
export interface SafeChangeMeta {
  riskScore: number;
  riskReasons: string[];
  rollbackScript: string;
  rollbackLevel: "none" | "sample" | "changedFields" | "full";
}

export type VqbCombinator = "and" | "or" | "nor";

export interface VqbGroup {
  kind: "group";
  combinator: VqbCombinator;
  children: VqbNode[];
}

export interface VqbCondition {
  kind: "condition";
  field: string;
  operator: string;
  value: unknown;
  enabled: boolean;
}

export type VqbNode = VqbGroup | VqbCondition;

export interface VqbTranslateRequest {
  node: VqbNode;
}

export interface FindRequest {
  connectionId: string;
  database: string;
  collection: string;
  filterJson: string;
  projectionJson?: string | null;
  sortJson?: string | null;
  limit?: number | null;
  skip?: number | null;
}

/**
 * Paged find request. Uses skip/limit paging: `skip = (page - 1) *
 * pageSize`. `sortJson` is honored as-is; when absent the driver's natural
 * order is used. Memory is bounded per page (`pageSize`), not by total
 * collection size.
 */
export interface FindPageRequest {
  connectionId: string;
  database: string;
  collection: string;
  filterJson: string;
  projectionJson?: string | null;
  sortJson?: string | null;
  /** 1-based page number (default 1). */
  page?: number;
  /** Page size (default 50, max 1000). Bounds memory per page, not total. */
  pageSize?: number | null;
  /** Count strategy; default `estimated`. */
  countMode?: CountMode;
}

export interface AggregateRequest {
  connectionId: string;
  database: string;
  collection: string;
  pipelineJson: string;
  limit?: number | null;
}

/**
 * Paged aggregation request. Aggregation output has no guaranteed `_id`
 * order, so paging is skip/limit only (`$skip`/`$limit` appended to the
 * pipeline) — no keyset. `countMode: "exact"` runs the pipeline again with a
 * `$count` stage (expensive, opt-in); `estimated` is treated as `none` since
 * a collection-size estimate is meaningless for pipeline output.
 */
export interface AggregatePageRequest {
  connectionId: string;
  database: string;
  collection: string;
  pipelineJson: string;
  /** 1-based page number (default 1). */
  page?: number;
  /** Page size (default 50, max 1000). */
  pageSize?: number | null;
  /** Count strategy; default `none` for aggregation. */
  countMode?: CountMode;
}

export interface CountRequest {
  connectionId: string;
  database: string;
  collection: string;
  filterJson?: string | null;
}

export interface CreateIndexRequest {
  connectionId: string;
  database: string;
  collection: string;
  name: string;
  keyJson: string;
  unique: boolean;
  sparse: boolean;
  hidden: boolean;
  ttlSeconds?: number | null;
  partialFilterExpressionJson?: string | null;
  collation?: CollationDto | null;
  wildcardProjectionJson?: string | null;
}

export interface ExplainRequest {
  connectionId: string;
  database: string;
  collection: string;
  filterJson: string;
}

export interface InsertRequest {
  connectionId: string;
  database: string;
  collection: string;
  documentJson: string;
}

export interface UpdateRequest {
  connectionId: string;
  database: string;
  collection: string;
  filterJson: string;
  updateJson: string;
  multi: boolean;
  upsert: boolean;
  safeChangeMeta?: SafeChangeMeta;
}

/** Result of an update operation. `matchedCount` distinguishes a true
 * no-match (filter missed — e.g. an `_id` round-trip bug) from a no-op
 * match (doc matched, value unchanged, `modifiedCount` 0). */
export interface UpdateResult {
  matchedCount: number;
  modifiedCount: number;
}

export interface ReplaceRequest {
  connectionId: string;
  database: string;
  collection: string;
  filterJson: string;
  replacementJson: string;
  upsert: boolean;
  safeChangeMeta?: SafeChangeMeta;
}

export interface ReplaceResult {
  matchedCount: number;
  modifiedCount: number;
  upsertedId: string | null;
}

export interface InsertManyRequest {
  connectionId: string;
  database: string;
  collection: string;
  documentsJson: string;
}

export interface InsertManyResult {
  insertedCount: number;
  insertedIds: string[];
}

export interface PreviewRequest {
  connectionId: string;
  database: string;
  collection: string;
  filterJson?: string | null;
}

export interface PreviewUpdateRequest {
  connectionId: string;
  database: string;
  collection: string;
  filterJson?: string | null;
  updateJson: string;
}

/** Audit log status snapshot. */
export interface AuditStatus {
  rootHex: string;
  leafCount: number;
  eventCount: number;
  treeHeight: number;
  domains: AuditDomain[];
}

export interface AuditDomain {
  deploymentId: string;
  database: string;
  eventCount: number;
}

/** A retained commitment to a logically pruned audit domain segment. */
export interface DomainRetentionRoot {
  rootHex: string;
  eventCount: number;
  maxIndex: number;
  prunedAt: string;
}

/** A single audit domain plus its secondary Merkle root and status. */
export interface DomainRootInfo {
  deploymentId: string;
  database: string;
  rootHex: string;
  eventCount: number;
  legalHold: boolean;
  retainedRoots: DomainRetentionRoot[];
}

/** A selective-disclosure inclusion proof against one domain's root. */
export interface DomainProofResult {
  deploymentId: string;
  database: string;
  position: number;
  leafHex: string;
  rootHex: string;
  pathElements: string[];
  pathIndices: number[];
}

/** The aggregation super-root over every per-domain root. */
export interface DomainSuperRootResult {
  superRootHex: string;
  domains: DomainRootInfo[];
}

/** An inclusion proof that a domain root is part of the super-root. */
export interface DomainSuperProofResult {
  deploymentId: string;
  database: string;
  domainRootHex: string;
  superRootHex: string;
  position: number;
  leafHex: string;
  pathElements: string[];
  pathIndices: number[];
}

/** An audit event recorded in the ZK audit log. */
export interface AuditEvent {
  index: number;
  leafHex: string;
  operation: string;
  database: string;
  collection: string;
  deploymentId: string;
  sequence: number;
  timestamp: string;
}

/** Soroban proof result for on-chain verification. */
export interface ProofResult {
  rootHex: string;
  leafHex: string;
  leafIndex: number;
  proof: { a: string; b: string; c: string };
  vk: {
    alpha: string;
    beta: string;
    gamma: string;
    delta: string;
    ic: string[];
  };
  pubSignals: string[];
  /** Stellar network the proof is anchored to (testnet or mainnet). */
  network: "testnet" | "mainnet";
  /** Soroban contract ID where the batch root is committed. */
  contractId: string;
  /** On-chain transaction hash that committed this batch's root. */
  txHash: string;
}

/** Result of committing a root to Stellar. */
export interface CommitResult {
  sequence: number;
  txHash: string;
  rootHex: string;
}

/** The latest committed root from Stellar. */
export interface OnChainRoot {
  sequence: number;
  rootHex: string;
  timestamp: number;
  metadata: string;
}

export interface Epoch {
  epochNumber: number;
  startIndex: number;
  endIndex: number | null;
  rootHex: string | null;
  eventCount: number;
  committed: boolean;
  committedAt: string | null;
  txHash: string | null;
}

// ─── Phase 3: Reader mode, IPFS, RPC, Attestation ───────────────────

/** Result of reader-mode verification against on-chain roots. */
export interface VerificationReport {
  onchainRootFound: boolean;
  onchainRoot: OnChainRoot | null;
  localRootHex: string;
  commitmentEventIndex: number | null;
  totalEvents: number;
  verifiedEvents: number;
  eventsAfterCommitment: number;
  chainIntact: boolean;
  tamperDetected: boolean;
  summary: string;
}

/** One persisted reader-mode verification run. */
export interface VerificationRecord {
  runAt: number;
  report: VerificationReport;
}

/** Result of publishing an epoch batch to IPFS. */
export interface IpfsPublishResult {
  cid: string;
  epochNumber: number;
  eventCount: number;
  batchSizeBytes: number;
  gatewayUrl: string;
}

/** Onboarding status — which components are already provisioned. */
export interface OnboardingStatus {
  hasKeypair: boolean;
  hasPinata: boolean;
  isComplete: boolean;
}

/** Which audit experience the user selected. */
export type AuditMode = "dev" | "production";

/** Which Stellar network to anchor commitments to. */
export type AuditNetwork = "testnet" | "mainnet";

/** The full audit mode configuration. */
export interface AuditModeConfig {
  mode: AuditMode;
  network: AuditNetwork;
  testnetContractId: string;
  mainnetContractId: string;
  mainnetRpcUrl: string;
  hasProductionKeypair: boolean;
}

/** Result of provisioning a per-user testnet commitment contract. */
export interface AuditContractProvisionResult {
  accountId: string;
  contractId: string;
  reused: boolean;
  wasmHashHex: string | null;
  uploadTxHash: string | null;
  createTxHash: string | null;
}

/** Dev-mode prerequisite check result. */
export interface DevPrerequisites {
  dockerInstalled: boolean;
  dockerComposeAvailable: boolean;
  composeFilePresent: boolean;
  portsFree: boolean;
  publisherPortFree: boolean;
  attesterPortFree: boolean;
  readerPortFree: boolean;
  auditStackRunning: boolean;
  dockerDaemonRunning: boolean;
  envAuditPresent: boolean;
  attesterKeyPresent: boolean;
  auditConfigured: boolean;
  summary: string;
}

/** Parameters for the non-interactive setup wizard. */
export interface DevSetupParams {
  network?: string;
  pinataApiKey?: string;
  pinataApiSecret?: string;
  pinataGatewayUrl?: string;
  publisherSecretKey?: string;
  attesterSecretKey?: string;
  contractId?: string;
  overwrite?: boolean;
  /**
   * MongoDB deployment the publisher should watch (change stream). When set,
   * it is persisted into `.env.audit`. Must be a replica set or sharded
   * cluster. Leave empty to watch the bundled demo replica set.
   */
  publisherMongoUri?: string;
  /**
   * Independent MongoDB member the attester/reader read from for oplog
   * verification. For a real trust anchor this should be a replica member the
   * operator does not control. Leave empty to fall back to the publisher URI
   * (functional but not independent).
   */
  attesterMongoUri?: string;
  /** Setup role: "all" (Dev Mode), "publisher", or "attester". */
  role?: string;
}

/** Result of the non-interactive setup wizard (log is secret-redacted). */
export interface DevSetupResult {
  success: boolean;
  log: string;
  envAuditPresent: boolean;
  attesterKeyPresent: boolean;
}

/** Public Stellar addresses of the dev-stack publisher and attester. */
export interface DevStackIdentities {
  publisherAddress: string;
  attesterAddress: string;
  contractId: string;
}

/** One audit-stack container. */
export interface DevStackService {
  name: string;
  state: string;
  ports: string;
}

/** Audit-stack container status. */
export interface DevStackStatus {
  running: boolean;
  services: DevStackService[];
  /**
   * The publisher's configured MongoDB URI (from `.env.audit`), or null when
   * the bundled demo replica set default is in effect.
   */
  publisherMongoUri?: string | null;
}

/** A registered publisher for threshold attestation. */
export interface Publisher {
  publicKey: string;
  name: string;
  registeredAt: string;
}

/** An attestation of an epoch root by a publisher. */
export interface Attestation {
  epochNumber: number;
  rootHex: string;
  publisherPublicKey: string;
  signature: string;
  submittedAt: string;
}

/** Threshold attestation status for an epoch. */
export interface AttestationStatus {
  epochNumber: number;
  rootHex: string;
  threshold: number;
  totalPublishers: number;
  validAttestations: number;
  thresholdMet: boolean;
  attestedBy: string[];
  pending: string[];
}

/** On-chain oplog commitment for an epoch (from the Soroban contract). */
export interface OnChainOplogCommitment {
  sequence: number;
  oplogRootHex: string;
  oplogStartTs: number;
  oplogEndTs: number;
  oplogEntryCount: number;
}

/** Independent on-chain attestation verdict from the Soroban contract. */
export interface OnChainAttestationVerification {
  sequence: number;
  oplogRootHex: string;
  attestationCount: number;
  authorizedCount: number;
  threshold: number;
  allMatch: boolean;
  /** "verified" | "threshold_not_met" | "unauthorized_attester" | "no_attestations" */
  verdict: string;
}

/** Result of the oplog integrity three-way compare. */
export interface OplogIntegrityReport {
  sequence: number;
  onChainOplogRoot: string;
  auditorOplogRoot: string | null;
  oplogEntryCount: number | null;
  allMatch: boolean;
  onChainMatchesAuditor: boolean;
  /** "complete", "mismatch", "stale", "no_commitment", "no_oplog_commitment", "contract_outdated", or "error" */
  verdict: string;
  explanation: string;
  alerts: string[];
}

const commands = {
  ping: (message: string) => invoke<string>("ping", { message }),
  appInfo: () => invoke<AppInfo>("app_info"),
  getSettings: () => invoke<AppSettings>("get_settings"),
  updateSettings: (settings: AppSettings) =>
    invoke<void>("update_settings", { settings }),

  // --- ZK Audit ---
  auditGetStatus: (deploymentId?: string | null, database?: string | null) =>
    invoke<AuditStatus>("audit_get_status", { deploymentId, database }),
  auditListEvents: (deploymentId?: string | null, database?: string | null) =>
    invoke<AuditEvent[]>("audit_list_events", { deploymentId, database }),
  auditGetRoot: () => invoke<string>("audit_get_root"),
  auditGenerateProof: (
    index: number,
    r1csPath?: string,
    wasmPath?: string,
    provingKeyPath?: string,
  ) =>
    invoke<ProofResult>("audit_generate_proof", {
      index,
      r1csPath,
      wasmPath,
      provingKeyPath,
    }),
  auditVerifyProofOnchain: (proof: {
    rootHex: string;
    leafHex: string;
    proofA: string;
    proofB: string;
    proofC: string;
  }) =>
    invoke<{ txHash: string; verified: boolean }>(
      "audit_verify_proof_onchain",
      proof,
    ),
  auditRecordEvent: (
    operation: string,
    database: string,
    collection: string,
    payload: string,
    deploymentId?: string | null,
  ) =>
    invoke<number>("audit_record_event", {
      operation,
      database,
      collection,
      deploymentId,
      payload,
    }),
  auditCommitRoot: (metadata?: string) =>
    invoke<CommitResult>("audit_commit_root", { metadata }),
  auditGetOnchainRoot: () =>
    invoke<OnChainRoot | null>("audit_get_onchain_root"),

  // --- ZK Audit: Epoch management ---
  auditListEpochs: () => invoke<Epoch[]>("audit_list_epochs"),
  auditCurrentEpoch: () => invoke<Epoch>("audit_current_epoch"),
  auditCloseEpoch: () => invoke<Epoch>("audit_close_epoch"),
  auditMarkEpochCommitted: (epochNumber: number, txHash: string) =>
    invoke<void>("audit_mark_epoch_committed", { epochNumber, txHash }),
  auditResetData: () => invoke<void>("audit_reset_data"),

  // --- ZK Audit: Phase 2 — per-domain segmentation ---
  auditListDomains: () => invoke<DomainRootInfo[]>("audit_list_domains"),
  auditGetDomainRoot: (deploymentId: string, database: string) =>
    invoke<DomainRootInfo>("audit_get_domain_root", { deploymentId, database }),
  auditGetDomainSuperRoot: () =>
    invoke<DomainSuperRootResult>("audit_get_domain_super_root"),
  auditGenerateDomainProof: (
    deploymentId: string,
    database: string,
    position: number,
  ) =>
    invoke<DomainProofResult>("audit_generate_domain_proof", {
      deploymentId,
      database,
      position,
    }),
  auditGenerateDomainSuperProof: (deploymentId: string, database: string) =>
    invoke<DomainSuperProofResult>("audit_generate_domain_super_proof", {
      deploymentId,
      database,
    }),
  auditSetLegalHold: (deploymentId: string, database: string, hold: boolean) =>
    invoke<void>("audit_set_legal_hold", { deploymentId, database, hold }),
  auditPruneDomain: (deploymentId: string, database: string) =>
    invoke<DomainRetentionRoot | null>("audit_prune_domain", {
      deploymentId,
      database,
    }),

  // --- ZK Audit: Phase 3 — Reader mode, IPFS, RPC, Attestation ---
  auditVerifyReaderMode: () =>
    invoke<VerificationReport>("audit_verify_reader_mode"),
  auditListVerificationHistory: () =>
    invoke<VerificationRecord[]>("audit_list_verification_history"),
  auditPublishEpochToIpfs: (epochNumber: number, apiUrl?: string) =>
    invoke<IpfsPublishResult>("audit_publish_epoch_to_ipfs", {
      epochNumber,
      apiUrl,
    }),
  auditGetIpfsCid: (epochNumber: number) =>
    invoke<string | null>("audit_get_ipfs_cid", { epochNumber }),
  auditCheckIpfsDaemon: (apiUrl?: string) =>
    invoke<boolean>("audit_check_ipfs_daemon", { apiUrl }),
  auditGetOnchainRootRpc: (rpcUrl?: string) =>
    invoke<OnChainRoot | null>("audit_get_onchain_root_rpc", { rpcUrl }),
  auditAddPublisher: (publicKey: string, name: string) =>
    invoke<Publisher>("audit_add_publisher", { publicKey, name }),
  auditRemovePublisher: (publicKey: string) =>
    invoke<void>("audit_remove_publisher", { publicKey }),
  auditListPublishers: () => invoke<Publisher[]>("audit_list_publishers"),
  auditSetAttestationThreshold: (threshold: number) =>
    invoke<void>("audit_set_attestation_threshold", { threshold }),
  auditGetAttestationThreshold: () =>
    invoke<number>("audit_get_attestation_threshold"),
  auditSubmitAttestation: (
    epochNumber: number,
    rootHex: string,
    publisherPublicKey: string,
    signatureHex: string,
  ) =>
    invoke<Attestation>("audit_submit_attestation", {
      epochNumber,
      rootHex,
      publisherPublicKey,
      signatureHex,
    }),
  auditListAttestations: (epochNumber: number) =>
    invoke<Attestation[]>("audit_list_attestations", { epochNumber }),
  auditGetAttestationStatus: (epochNumber: number, rootHex: string) =>
    invoke<AttestationStatus>("audit_get_attestation_status", {
      epochNumber,
      rootHex,
    }),

  // --- On-chain (independent) attestation ---
  auditAuthorizeOnchainAttester: (
    stellarAddress: string,
    ed25519PubkeyHex: string,
  ) =>
    invoke<void>("audit_authorize_onchain_attester", {
      stellarAddress,
      ed25519PubkeyHex,
    }),
  auditRevokeOnchainAttester: (stellarAddress: string) =>
    invoke<void>("audit_revoke_onchain_attester", { stellarAddress }),
  auditSetOnchainThreshold: (threshold: number) =>
    invoke<void>("audit_set_onchain_threshold", { threshold }),
  auditGetOnchainThreshold: () =>
    invoke<number>("audit_get_onchain_threshold"),
  auditVerifyOnchainAttestation: (sequence: number) =>
    invoke<OnChainAttestationVerification>("audit_verify_onchain_attestation", {
      sequence,
    }),

  // --- ZK Audit: Oplog completeness verification ---
  auditVerifyOplogIntegrity: (connectionId: string, rpcUrl?: string) =>
    invoke<OplogIntegrityReport>("audit_verify_oplog_integrity", {
      connectionId,
      rpcUrl,
    }),
  auditGetOplogCommitment: (sequence: number) =>
    invoke<OnChainOplogCommitment | null>("audit_get_oplog_commitment", {
      sequence,
    }),

  // --- ZK Audit: Dev mode onboarding ---
  auditCheckOnboarding: () =>
    invoke<OnboardingStatus>("audit_check_onboarding"),
  auditSavePinataConfig: (apiKey: string, apiSecret: string) =>
    invoke<boolean>("audit_save_pinata_config", { apiKey, apiSecret }),
  auditTestPinataConnection: (apiKey: string, apiSecret: string) =>
    invoke<boolean>("audit_test_pinata_connection", { apiKey, apiSecret }),
  auditGenerateStellarAccount: () =>
    invoke<string>("audit_generate_stellar_account"),
  auditProvisionTestnetContract: () =>
    invoke<AuditContractProvisionResult>("audit_provision_testnet_contract"),
  auditCheckReplicaSet: (connectionId: string) =>
    invoke<boolean>("audit_check_replica_set", { connectionId }),
  auditCommitRootNative: (metadata?: string, connectionId?: string) =>
    invoke<CommitResult>("audit_commit_root_native", { metadata, connectionId }),
  auditPublishEpochToPinata: (epochNumber: number) =>
    invoke<IpfsPublishResult>("audit_publish_epoch_to_pinata", {
      epochNumber,
    }),

  // --- ZK Audit: Production mode (in-app pipeline, user's own keys) ---
  auditCommitRootProduction: (metadata?: string, connectionId?: string) =>
    invoke<CommitResult>("audit_commit_root_production", { metadata, connectionId }),

  // --- ZK Audit: Mode selection (dev / production) ---
  auditGetModeConfig: () => invoke<AuditModeConfig>("audit_get_mode_config"),
  auditSetAuditMode: (mode: AuditMode) =>
    invoke<void>("audit_set_audit_mode", { mode }),
  auditSetProductionNetwork: (
    network: AuditNetwork,
    contractId: string,
    rpcUrl: string,
  ) =>
    invoke<void>("audit_set_production_network", {
      network,
      contractId,
      rpcUrl,
    }),
  auditImportProductionKeypair: (secretKey: string) =>
    invoke<string>("audit_import_production_keypair", { secretKey }),
  auditClearProductionKeypair: () =>
    invoke<void>("audit_clear_production_keypair"),
  auditGetActiveAccount: () =>
    invoke<string | null>("audit_get_active_account"),

  // --- ZK Audit: Dev mode Docker orchestration ---
  auditCheckDevPrerequisites: () =>
    invoke<DevPrerequisites>("audit_check_dev_prerequisites"),
  auditDevStackStatus: () =>
    invoke<DevStackStatus>("audit_dev_stack_status"),
  auditDevStackUp: () => invoke<string>("audit_dev_stack_up"),
  auditDevStackDown: () => invoke<string>("audit_dev_stack_down"),
  auditDevStackResetData: () => invoke<string>("audit_dev_stack_reset_data"),
  auditDevStackLogs: (tail?: number) =>
    invoke<string>("audit_dev_stack_logs", { tail }),
  auditDevStackSetup: (params: DevSetupParams) =>
    invoke<DevSetupResult>("audit_dev_stack_setup", { params }),
  auditDevStackIdentities: () =>
    invoke<DevStackIdentities | null>("audit_dev_stack_identities"),

  // --- ZK Audit: Dev mode audit service HTTP proxy (to docker) ---
  auditDevProxyGet: (port: number, path: string) =>
    invoke<unknown>("audit_dev_proxy_get", { port, path }),
  auditDevProxyPost: (port: number, path: string, body?: unknown) =>
    invoke<unknown>("audit_dev_proxy_post", { port, path, body }),

  listProfiles: () => invoke<ProfileSummary[]>("list_profiles"),
  saveProfile: (request: SaveProfileRequest) =>
    invoke<ProfileSummary>("save_profile", { request }),
  deleteProfile: (id: string) => invoke<void>("delete_profile", { id }),
  testProfile: (request: SaveProfileRequest) =>
    invoke<TestResult>("test_profile", { request }),
  openConnection: (profileId: string, secretOverride?: string) =>
    invoke<ConnectionHandle>("open_connection", {
      profileId,
      secretOverride,
    }),
  closeConnection: (connectionId: string) =>
    invoke<void>("close_connection", { connectionId }),
  listActiveConnections: () =>
    invoke<ConnectionDescriptor[]>("list_active_connections"),
  resolveProfileUri: (id: string) =>
    invoke<string>("resolve_profile_uri", { id }),

  listDatabases: (connectionId: string) =>
    invoke<DatabaseSummary[]>("list_databases", { connectionId }),
  listCollections: (connectionId: string, database: string) =>
    invoke<CollectionSummary[]>("list_collections", { connectionId, database }),
  collectionStats: (
    connectionId: string,
    database: string,
    collection: string,
  ) =>
    invoke<CollectionStats>("collection_stats", {
      connectionId,
      database,
      collection,
    }),
  findDocuments: (request: FindRequest) =>
    invoke<DocumentPage>("find_documents", { request }),
  findPage: (request: FindPageRequest) =>
    invoke<DocumentPage>("find_page", { request }),
  aggregateDocuments: (request: AggregateRequest) =>
    invoke<DocumentPage>("aggregate_documents", { request }),
  aggregatePage: (request: AggregatePageRequest) =>
    invoke<DocumentPage>("aggregate_page", { request }),
  countDocuments: (request: CountRequest) =>
    invoke<number>("count_documents", { request }),
  listIndexes: (connectionId: string, database: string, collection: string) =>
    invoke<IndexInfo[]>("list_indexes", {
      connectionId,
      database,
      collection,
    }),
  createIndex: (request: CreateIndexRequest) =>
    invoke<string>("create_index", { request }),
  dropIndex: (
    connectionId: string,
    database: string,
    collection: string,
    name: string,
  ) =>
    invoke<void>("drop_index", { connectionId, database, collection, name }),
  indexStats: (connectionId: string, database: string, collection: string) =>
    invoke<IndexStats[]>("index_stats", { connectionId, database, collection }),
  explainFind: (request: ExplainRequest) =>
    invoke<ExplainResult>("explain_find", { request }),
  explainAggregate: (
    connectionId: string,
    database: string,
    collection: string,
    pipelineJson: string,
  ) =>
    invoke<ExplainResult>("explain_aggregate", {
      connectionId,
      database,
      collection,
      pipelineJson,
    }),
  sampleSchema: (connectionId: string, database: string, collection: string) =>
    invoke<SchemaReport>("sample_schema", {
      connectionId,
      database,
      collection,
    }),
  sampleShape: (
    connectionId: string,
    database: string,
    collection: string,
    sampleSize?: number,
  ) =>
    invoke<CollectionShape>("sample_shape", {
      request: {
        connectionId,
        database,
        collection,
        sampleSize: sampleSize ?? null,
      },
    }),
  insertDocument: (request: InsertRequest) =>
    invoke<string>("insert_document", { request }),
  insertManyDocuments: (request: InsertManyRequest) =>
    invoke<InsertManyResult>("insert_many_documents", { request }),
  updateDocuments: (request: UpdateRequest) =>
    invoke<UpdateResult>("update_documents", { request }),
  replaceDocument: (request: ReplaceRequest) =>
    invoke<ReplaceResult>("replace_document", { request }),
  deleteDocuments: (
    connectionId: string,
    database: string,
    collection: string,
    filterJson: string,
    safeChangeMeta?: SafeChangeMeta,
  ) =>
    invoke<number>("delete_documents", {
      connectionId,
      database,
      collection,
      filterJson,
      safeChangeMeta: safeChangeMeta ?? null,
    }),
  previewDelete: (request: PreviewRequest) =>
    invoke<number>("preview_delete", { request }),
  previewUpdate: (request: PreviewUpdateRequest) =>
    invoke<number>("preview_update", { request }),
  safeChangePreview: (request: SafeChangePreviewRequest) =>
    invoke<SafeChangePreview>("safe_change_preview", { request }),
  translateVqb: (request: VqbTranslateRequest) =>
    invoke<Record<string, unknown>>("translate_vqb", { request }),
  translateSql: (database: string, sql: string) =>
    invoke<SqlTranslation>("translate_sql", { database, sql }),
  generatePipelineCode: (request: {
    database: string;
    collection: string;
    pipeline: unknown[];
    language: string;
    profileName?: string;
    authMechanism?: string;
    uri: string;
  }) => invoke<string>("generate_pipeline_code", { request }),
  evalShell: (request: {
    connectionId: string;
    script: string;
    activeDatabase?: string;
    fallbackDatabase?: string;
  }) => invoke<ShellResponse>("eval_shell", { request }),
  exportDocuments: (request: ExportRequest) =>
    invoke<ExportResult>("export_documents", { request }),
  cancelImportExport: (jobId: string) =>
    invoke<boolean>("cancel_import_export", { jobId }),
  copyDocuments: (request: CopyRequest) =>
    invoke<CopyResult>("copy_documents", { request }),
  previewImport: (request: ImportRequest) =>
    invoke<PreviewImportResult>("preview_import", { request }),
  runImport: (request: ImportRequest) =>
    invoke<ImportResult>("run_import", { request }),

  // --- Jobs ---
  listJobs: (request: JobFilterRequest) =>
    invoke<JobListResponse>("list_jobs", { request }),
  getJob: (jobId: string) =>
    invoke<JobDetailResponse>("get_job", { jobId }),
  cancelJob: (jobId: string) =>
    invoke<boolean>("cancel_job", { jobId }),
  deleteJob: (jobId: string) =>
    invoke<boolean>("delete_job", { jobId }),
  rerunJob: (jobId: string) =>
    invoke<JobMeta>("rerun_job", { jobId }),
  updateSchedule: (request: { jobId: string; cron: string; enabled: boolean; retentionCount?: number | null; profileId?: string | null }) =>
    invoke<JobMeta>("update_schedule", { request }),

  // --- Dump / Restore ---
  dumpDatabase: (request: DumpRequest) =>
    invoke<DumpResult>("dump_database", { request }),
  previewArchive: (sourceDir: string) =>
    invoke<ArchivePreviewEntry[]>("preview_archive", { sourceDir }),
  restoreDatabase: (request: RestoreRequest) =>
    invoke<RestoreResult>("restore_database", { request }),
  scanDataModel: (request: ScanScopeRequest) =>
    invoke<DataModelGraph>("scan_data_model", { request }),
  getDataModel: (database: string) =>
    invoke<DataModelGraph | null>("get_data_model", { database }),
  updateRelationship: (
    database: string,
    edgeId: string,
    overrides: { confirmed?: boolean; hidden?: boolean },
  ) =>
    invoke<DataModelGraph>("update_relationship", {
      database,
      edgeId,
      confirmed: overrides.confirmed ?? null,
      hidden: overrides.hidden ?? null,
    }),

  shellAutocomplete: (request: {
    connectionId: string;
    textBeforeCursor: string;
    activeDatabase?: string;
    fallbackDatabase?: string;
  }) => invoke<AutocompleteResponse>("shell_autocomplete", { request }),
};

/** One output line from the mongo shell. */
export type ShellOutput =
  | { kind: "text"; value: string }
  | { kind: "json"; value: unknown }
  | { kind: "error"; value: string }
  | { kind: "table"; value: ShellTable };

export interface ShellTable {
  columns: string[];
  rows: unknown[][];
  executionMs: number;
}

export interface ShellResponse {
  outputs: ShellOutput[];
  lastPipeline: unknown[] | null;
  lastCollection: string | null;
  lastDatabase: string | null;
  activeDatabase: string;
  executionMs: number;
  operations?: ShellOperation[];
}

export interface ShellOperation {
  kind: import("./timeline").OperationKind;
  database: string;
  collection: string;
  queryJson: string | null;
  updateJson: string | null;
  matchedCount: number | null;
  modifiedCount: number | null;
  insertedCount: number | null;
  deletedCount: number | null;
  executionMs: number | null;
  errored: boolean;
  errorMessage: string | null;
}

export type CompletionKind =
  | { kind: "collections" }
  | { kind: "methods"; collection: string }
  | { kind: "fields"; collection: string }
  | { kind: "operators"; method: string }
  | { kind: "databases" }
  | { kind: "globals" }
  | { kind: "none" };

export interface CompletionItem {
  label: string;
  detail: string;
}

export interface AutocompleteResponse {
  kind: CompletionKind;
  items: CompletionItem[];
}

export default commands;
