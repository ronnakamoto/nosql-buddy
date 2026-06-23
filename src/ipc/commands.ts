// Typed IPC command wrappers for the frontend.
// Every command returns `Promise<T>` and the actual type matches the
// matching `#[tauri::command]` in src-tauri/src/commands/. Any error
// the Rust side produces is serialized as `{ kind, message }`.

import { invoke } from "@tauri-apps/api/core";

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
}

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

export interface SqlTranslation {
  database: string;
  collection: string;
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

export interface AppInfo {
  platform: string;
  arch: string;
  tauriVersion: string;
  appName: string;
  appVersion: string;
}

export interface AppSettings {
  theme: "system" | "light" | "dark";
  lastConnectionId: string | null;
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

export interface AggregateRequest {
  connectionId: string;
  database: string;
  collection: string;
  pipelineJson: string;
  limit?: number | null;
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
}

export interface PreviewRequest {
  connectionId: string;
  database: string;
  collection: string;
  filterJson?: string | null;
}

/** Audit log status snapshot. */
export interface AuditStatus {
  rootHex: string;
  leafCount: number;
  eventCount: number;
  treeHeight: number;
}

/** An audit event recorded in the ZK audit log. */
export interface AuditEvent {
  index: number;
  leafHex: string;
  operation: string;
  database: string;
  collection: string;
  timestamp: string;
}

/** Soroban proof result for on-chain verification. */
export interface ProofResult {
  rootHex: string;
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
}

const commands = {
  ping: (message: string) => invoke<string>("ping", { message }),
  appInfo: () => invoke<AppInfo>("app_info"),
  getSettings: () => invoke<AppSettings>("get_settings"),
  updateSettings: (settings: AppSettings) =>
    invoke<void>("update_settings", { settings }),

  // --- ZK Audit ---
  auditGetStatus: () => invoke<AuditStatus>("audit_get_status"),
  auditListEvents: () => invoke<AuditEvent[]>("audit_list_events"),
  auditGetRoot: () => invoke<string>("audit_get_root"),
  auditGenerateProof: (
    index: number,
    r1csPath: string,
    wasmPath: string,
  ) =>
    invoke<ProofResult>("audit_generate_proof", {
      index,
      r1csPath,
      wasmPath,
    }),
  auditRecordEvent: (
    operation: string,
    database: string,
    collection: string,
    payload: string,
  ) =>
    invoke<number>("audit_record_event", {
      operation,
      database,
      collection,
      payload,
    }),

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
  aggregateDocuments: (request: AggregateRequest) =>
    invoke<DocumentPage>("aggregate_documents", { request }),
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
  insertDocument: (request: InsertRequest) =>
    invoke<string>("insert_document", { request }),
  updateDocuments: (request: UpdateRequest) =>
    invoke<number>("update_documents", { request }),
  deleteDocuments: (
    connectionId: string,
    database: string,
    collection: string,
    filterJson: string,
  ) =>
    invoke<number>("delete_documents", {
      connectionId,
      database,
      collection,
      filterJson,
    }),
  previewDelete: (request: PreviewRequest) =>
    invoke<number>("preview_delete", { request }),
  previewUpdate: (request: PreviewRequest) =>
    invoke<number>("preview_update", { request }),
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
