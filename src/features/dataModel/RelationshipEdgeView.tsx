import {
  BaseEdge,
  EdgeLabelRenderer,
  getSmoothStepPath,
  type EdgeProps,
} from "@xyflow/react";
import type { RelationshipEdge } from "../../ipc/commands";

export interface RelationshipEdgeData {
  edge: RelationshipEdge;
}

export function RelationshipEdgeView({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
}: EdgeProps & { data: RelationshipEdgeData }) {
  const [edgePath, labelX, labelY] = getSmoothStepPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
    borderRadius: 8,
  });

  const edge = data.edge;
  const isDashed = edge.confidence < 0.75;
  const isDotted = edge.confidence < 0.4;
  const stroke =
    edge.confidence >= 0.75
      ? "var(--accent-500)"
      : edge.confidence >= 0.4
        ? "var(--ink-muted)"
        : "var(--ink-faint)";

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        style={{
          stroke,
          strokeWidth: edge.confidence >= 0.75 ? 2 : 1,
          strokeDasharray: isDotted ? "2,4" : isDashed ? "6,4" : undefined,
        }}
      />
      <EdgeLabelRenderer>
        <div
          className="relationship-edge__label"
          style={{
            transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
          }}
        >
          {kindLabel(edge.kind)}
        </div>
      </EdgeLabelRenderer>
    </>
  );
}

function kindLabel(kind: string): string {
  switch (kind) {
    case "one-to-one":
      return "1:1";
    case "one-to-many":
      return "1:N";
    case "many-to-one":
      return "N:1";
    case "many-to-many":
      return "N:N";
    default:
      return kind;
  }
}
