import { useMemo } from "react";
import type {
  CollectionSummary,
  ConnectionHandle,
  DatabaseSummary,
  ProfileSummary,
} from "../ipc/commands";
import {
  Database,
  Layers,
  Server,
  ArrowRight,
  TrendingUp,
  PieChart,
  FileText,
} from "lucide-react";
import {
  DocumentsByDatabaseChart,
  StorageShareDonutChart,
  TopCollectionsChart,
  CollectionTypeChart,
} from "./OverviewCharts";

function isFiniteNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

function safeNum(v: unknown): number {
  return isFiniteNumber(v) ? v : 0;
}

function formatBytes(bytes: number | null | undefined): string {
  const n = safeNum(bytes);
  if (n <= 0) return "0 B";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatCount(n: number | null | undefined): string {
  const v = safeNum(n);
  if (v <= 0) return "0";
  if (v < 1000) return String(Math.floor(v));
  if (v < 1_000_000) return `${(v / 1000).toFixed(1)}k`;
  return `${(v / 1_000_000).toFixed(1)}M`;
}

function topologyLabel(topology: string | null | undefined): string {
  if (!topology) return "unknown";
  switch (topology) {
    case "replicaSet":
      return "Replica Set";
    case "sharded":
      return "Sharded Cluster";
    case "standalone":
      return "Standalone";
    default:
      return topology;
  }
}

export function ConnectionOverview({
  active,
  onOpenDatabase,
}: {
  active: {
    handle: ConnectionHandle;
    profile: ProfileSummary;
    databases: DatabaseSummary[];
    collections: Record<string, CollectionSummary[]>;
  };
  onOpenDatabase: (databaseName: string) => void;
}) {
  const { handle, databases, collections } = active;
  const serverInfo = handle.serverInfo;
  const safeDatabases = Array.isArray(databases) ? databases : [];
  const safeCollections = collections ?? {};

  const maxSize = useMemo(() => {
    return Math.max(1, ...safeDatabases.map((d) => safeNum(d?.sizeOnDisk)));
  }, [safeDatabases]);

  const dbList = useMemo(() => {
    return [...safeDatabases]
      .filter((d) => d && d.name)
      .sort((a, b) => safeNum(b?.sizeOnDisk) - safeNum(a?.sizeOnDisk));
  }, [safeDatabases]);

  return (
    <div className="overview">
      {/* Server header */}
      <header className="overview__header">
        <div className="overview__server-id">
          <Server size={18} className="overview__server-icon" aria-hidden="true" />
          <div className="overview__server-text">
            <h1 className="overview__server-name">{handle.name}</h1>
            <span className="overview__server-meta">
              {serverInfo?.host ?? "unknown host"}
            </span>
          </div>
        </div>
        <div className="overview__server-tags">
          <span className="overview__tag overview__tag--topology">
            {topologyLabel(serverInfo?.topology)}
          </span>
          {serverInfo?.version && (
            <span className="overview__tag">MongoDB {serverInfo.version}</span>
          )}
          {serverInfo?.isMaster != null && (
            <span
              className={
                "overview__tag overview__tag--" +
                (serverInfo.isMaster ? "primary" : "secondary")
              }
            >
              <span className="overview__status-dot" aria-hidden="true" />
              {serverInfo.isMaster ? "Writable Primary" : "Read-Only"}
            </span>
          )}
        </div>
      </header>

      {/* Charts */}
      <div className="overview__charts">
        <section className="overview__chart-panel">
          <h3 className="overview__chart-title">
            <FileText size={13} aria-hidden="true" />
            <span>Documents by Database</span>
          </h3>
          <DocumentsByDatabaseChart databases={safeDatabases} collections={safeCollections} />
        </section>
        <section className="overview__chart-panel">
          <h3 className="overview__chart-title">
            <PieChart size={13} aria-hidden="true" />
            <span>Storage Share</span>
          </h3>
          <StorageShareDonutChart databases={safeDatabases} />
        </section>
        <section className="overview__chart-panel">
          <h3 className="overview__chart-title">
            <TrendingUp size={13} aria-hidden="true" />
            <span>Top Collections by Documents</span>
          </h3>
          <TopCollectionsChart databases={safeDatabases} collections={safeCollections} />
        </section>
        <section className="overview__chart-panel">
          <h3 className="overview__chart-title">
            <PieChart size={13} aria-hidden="true" />
            <span>Collection Types</span>
          </h3>
          <CollectionTypeChart databases={safeDatabases} collections={safeCollections} />
        </section>
      </div>

      {/* Database grid */}
      <section className="overview__section">
        <div className="overview__section-head">
          <h2 className="overview__section-title">
            <Database size={14} aria-hidden="true" />
            <span>Databases</span>
          </h2>
          <span className="overview__section-count">{safeDatabases.length}</span>
        </div>
        {dbList.length === 0 ? (
          <div className="overview__empty">
            No databases found on this server.
          </div>
        ) : (
          <div className="overview__db-grid">
            {dbList.map((db, idx) => {
              const colls = safeCollections[db.name] ?? [];
              const dbDocCount = safeNum(db.documentCount) || colls.reduce((n, c) => n + safeNum(c?.documentCount), 0);
              const safeMax = maxSize > 0 ? maxSize : 1;
              const sizePct = Math.max(2, (safeNum(db.sizeOnDisk) / safeMax) * 100);
              return (
                <div
                  key={`${db.name}-${idx}`}
                  className="overview__db-card"
                  role="button"
                  tabIndex={0}
                  aria-label={`Open ${db.name} in Find`}
                  title={`Open ${db.name} in Find${colls.length ? ` (${colls[0].name})` : ""}`}
                  onClick={() => onOpenDatabase(db.name)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      onOpenDatabase(db.name);
                    }
                  }}
                >
                  <div className="overview__db-card-head">
                    <Database size={14} className="overview__db-icon" aria-hidden="true" />
                    <span className="overview__db-name">{db.name}</span>
                    <ArrowRight size={12} className="overview__db-arrow" aria-hidden="true" />
                  </div>
                  <div className="overview__db-stats">
                    <span className="overview__db-stat">
                      <Layers size={11} aria-hidden="true" />
                      {safeNum(db.collectionsCount) || colls.length} collections
                    </span>
                    <span className="overview__db-stat overview__db-stat--mono">
                      {formatCount(dbDocCount)} docs
                    </span>
                    <span className="overview__db-stat overview__db-stat--mono">
                      {formatBytes(db.sizeOnDisk)}
                    </span>
                  </div>
                  <div className="overview__db-bar" aria-hidden="true">
                    <div className="overview__db-bar-fill" style={{ width: `${sizePct}%` }} />
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}
