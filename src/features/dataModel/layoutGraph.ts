import Dagre from "@dagrejs/dagre";
import type { Edge, Node } from "@xyflow/react";

/** Shared node geometry. Kept here so the SVG/PNG exporter renders nodes at the
 * same dimensions React Flow + dagre lay them out at. */
export const NODE_WIDTH = 260;
export const HEADER_HEIGHT = 38;
export const FIELD_HEIGHT = 26;
export const FIELD_PADDING = 24;
export const VISIBLE_FIELDS = 8;
export const MORE_ROW_HEIGHT = 22;

export function nodeHeight(node: Node): number {
  const shape = (node.data as { shape?: { root?: { children?: unknown[] } } })?.shape;
  const fieldCount = Array.isArray(shape?.root?.children) ? shape.root.children.length : 0;
  const visible = Math.min(fieldCount, VISIBLE_FIELDS);
  const hasMore = fieldCount > VISIBLE_FIELDS ? MORE_ROW_HEIGHT : 0;
  return HEADER_HEIGHT + visible * FIELD_HEIGHT + hasMore + FIELD_PADDING;
}

export function layoutGraph(nodes: Node[], edges: Edge[]): { nodes: Node[]; edges: Edge[] } {
  const g = new Dagre.graphlib.Graph().setDefaultEdgeLabel(() => ({}));
  g.setGraph({ rankdir: "LR", nodesep: 80, ranksep: 180, marginx: 40, marginy: 40 });

  for (const node of nodes) {
    const height = nodeHeight(node);
    g.setNode(node.id, { width: NODE_WIDTH, height });
  }

  for (const edge of edges) {
    g.setEdge(edge.source, edge.target);
  }

  Dagre.layout(g);

  const positioned = nodes.map((node) => {
    const withPosition = g.node(node.id);
    const height = nodeHeight(node);
    return {
      ...node,
      position: {
        x: withPosition.x - NODE_WIDTH / 2,
        y: withPosition.y - height / 2,
      },
    };
  });

  return { nodes: positioned, edges };
}
