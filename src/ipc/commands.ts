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
  mainnetContractId: string;
  mainnetRpcUrl: string;
  hasProductionKeypair: boolean;
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
}

/** Result of the non-interactive setup wizard (log is secret-redacted). */
export interface DevSetupResult {
  success: boolean;
  log: string;
  envAuditPresent: boolean;
  attesterKeyPresent: boolean;
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
  auditGetStatus: () => invoke<AuditStatus>("audit_get_status"),
  auditListEvents: () => invoke<AuditEvent[]>("audit_list_events"),
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
    proofA: string;
    proofB: string;
    proofC: string;
    vkAlpha: string;
    vkBeta: string;
    vkGamma: string;
    vkDelta: string;
    vkIc: string[];
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
  auditResetData: () => invoke<void>("audit_reset_data"),

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
