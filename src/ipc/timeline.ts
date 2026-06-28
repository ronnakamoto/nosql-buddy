// Typed IPC wrappers for Data Timeline commands.

import { invoke } from "@tauri-apps/api/core";

export type OperationKind =
  | "find"
  | "aggregate"
  | "sql"
  | "explain"
  | "insertOne"
  | "insertMany"
  | "updateOne"
  | "updateMany"
  | "deleteOne"
  | "deleteMany"
  | "replaceOne"
  | "aggregationWrite"
  | "indexCreate"
  | "indexDrop"
  | "collectionCreate"
  | "collectionDrop"
  | "import"
  | "export"
  | "dump"
  | "restore";

export type RollbackLevel = "none" | "sample" | "changedFields" | "full";

export type ApprovalStatus =
  | "notRequired"
  | "pending"
  | "approved"
  | "rejected";

export interface TimelineEntry {
  id: string;
  profileId: string;
  connectionId: string;
  kind: OperationKind;
  database: string;
  collection: string;
  actor: string;
  environmentTag: string;
  queryJson: string | null;
  updateJson: string | null;
  matchedCount: number | null;
  modifiedCount: number | null;
  insertedCount: number | null;
  deletedCount: number | null;
  riskScore: number | null;
  riskReasons: string[] | null;
  approvalStatus: ApprovalStatus;
  reviewers: string[] | null;
  rollbackLevel: RollbackLevel;
  rollbackScript: string | null;
  rollbackArchivePath: string | null;
  notes: string | null;
  createdAt: string;
  executedAt: string | null;
  executionMs: number | null;
  errored: boolean;
  errorMessage: string | null;
  returnedCount: number | null;
}

export interface TimelineFilters {
  profileId: string;
  database?: string;
  collection?: string;
  kind?: OperationKind;
  from?: string;
  to?: string;
  limit?: number;
  errored?: boolean;
}

export async function listTimeline(filters: TimelineFilters): Promise<TimelineEntry[]> {
  return invoke<TimelineEntry[]>("list_timeline", {
    request: {
      profileId: filters.profileId,
      database: filters.database ?? null,
      collection: filters.collection ?? null,
      kind: filters.kind ?? null,
      from: filters.from ?? null,
      to: filters.to ?? null,
      limit: filters.limit ?? null,
      errored: filters.errored ?? null,
    },
  });
}

export async function getTimelineEntry(id: string): Promise<TimelineEntry | null> {
  return invoke<TimelineEntry | null>("get_timeline_entry", { id });
}

export async function addTimelineNote(id: string, note: string): Promise<boolean> {
  return invoke<boolean>("add_timeline_note", { id, note });
}

export async function deleteTimelineEntry(id: string): Promise<boolean> {
  return invoke<boolean>("delete_timeline_entry", { id });
}

export function operationKindLabel(kind: OperationKind): string {
  switch (kind) {
    case "find": return "Find";
    case "aggregate": return "Aggregate";
    case "sql": return "SQL";
    case "explain": return "Explain";
    case "insertOne": return "Insert One";
    case "insertMany": return "Insert Many";
    case "updateOne": return "Update One";
    case "updateMany": return "Update Many";
    case "deleteOne": return "Delete One";
    case "deleteMany": return "Delete Many";
    case "replaceOne": return "Replace One";
    case "aggregationWrite": return "Aggregation Write";
    case "indexCreate": return "Create Index";
    case "indexDrop": return "Drop Index";
    case "collectionCreate": return "Create Collection";
    case "collectionDrop": return "Drop Collection";
    case "import": return "Import";
    case "export": return "Export";
    case "dump": return "Dump";
    case "restore": return "Restore";
  }
}

export function approvalStatusLabel(status: ApprovalStatus): string {
  switch (status) {
    case "notRequired": return "Not Required";
    case "pending": return "Pending";
    case "approved": return "Approved";
    case "rejected": return "Rejected";
  }
}

export function rollbackLevelLabel(level: RollbackLevel): string {
  switch (level) {
    case "none": return "None";
    case "sample": return "Sample";
    case "changedFields": return "Changed Fields";
    case "full": return "Full";
  }
}
