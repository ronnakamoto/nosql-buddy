import Dagre from "@dagrejs/dagre";
import type { Edge, Node } from "@xyflow/react";

const NODE_WIDTH = 260;
const HEADER_HEIGHT = 38;
const FIELD_HEIGHT = 26;
const PADDING = 24;

function nodeHeight(node: Node): number {
  const shape = (node.data as { shape?: { root?: { children?: unknown[] } } })?.shape;
  const fieldCount = Array.isArray(shape?.root?.children) ? shape.root.children.length : 0;
  const visible = Math.min(fieldCount, 8);
  const hasMore = fieldCount > 8 ? 22 : 0;
  return HEADER_HEIGHT + visible * FIELD_HEIGHT + hasMore + PADDING;
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
