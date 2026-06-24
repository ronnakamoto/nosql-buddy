// Typed IPC command wrappers for the frontend.
// Every command returns `Promise<T>` and the actual type matches the
// matching `#[tauri::command]` in src-tauri/src/commands/. Any error
// the Rust side produces is serialized as `{ kind, message }`.

import { invoke } from "@tauri-apps/api/core";

/** Format a Tauri/Rust error for display. Errors are serialized as `{ kind, message }`. */
export function formatError(err: unknown): string {
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

/** Result of an update operation. `matchedCount` distinguishes a true
 * no-match (filter missed — e.g. an `_id` round-trip bug) from a no-op
 * match (doc matched, value unchanged, `modifiedCount` 0). */
export interface UpdateResult {
  matchedCount: number;
  modifiedCount: number;
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

/** Result of publishing an epoch batch to IPFS. */
export interface IpfsPublishResult {
  cid: string;
  epochNumber: number;
  eventCount: number;
  batchSizeBytes: number;
  gatewayUrl: string;
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

/** Result of the oplog integrity three-way compare. */
export interface OplogIntegrityReport {
  sequence: number;
  onChainOplogRoot: string;
  auditorOplogRoot: string | null;
  oplogEntryCount: number | null;
  allMatch: boolean;
  onChainMatchesAuditor: boolean;
  /** "complete", "mismatch", "stale", "no_commitment", "no_oplog_commitment", or "error" */
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
  auditGetStatus: () => invoke<AuditStatus>("audit_get_status"),
  auditListEvents: () => invoke<AuditEvent[]>("audit_list_events"),
  auditGetRoot: () => invoke<string>("audit_get_root"),
  auditGenerateProof: (
    index: number,
    r1csPath?: string,
    wasmPath?: string,
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

  // --- ZK Audit: Phase 3 — Reader mode, IPFS, RPC, Attestation ---
  auditVerifyReaderMode: () =>
    invoke<VerificationReport>("audit_verify_reader_mode"),
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
    invoke<UpdateResult>("update_documents", { request }),
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
