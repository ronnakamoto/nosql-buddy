import { useState } from "react";
import type { ProofResult } from "../ipc/commands";

/**
 * MerklePathViz — SVG visualization of a Merkle inclusion proof.
 *
 * Renders a simple leaf → root graph. The actual sibling path elements are
 * private circuit witness data and are never returned to the frontend, so
 * this only shows the two public signals the on-chain verifier checks:
 * `leafHex` (the audit-entry hash being proven included) and `rootHex` (the
 * committed Merkle root).
 */

// ─── Layout constants ────────────────────────────────────────────────────────

const NODE_W = 136;
const NODE_H = 32;
const H_GAP = 28;  // gap between levels (left→right)
const V_GAP = 14;  // gap between nodes at the same level

// ─── Types ───────────────────────────────────────────────────────────────────

type NodeKind = "leaf" | "path" | "root";

interface MerkleNode {
  id: string;
  label: string;
  fullHash: string;
  kind: NodeKind;
  x: number;
  y: number;
}

interface MerkleEdge {
  fromId: string;
  toId: string;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

function shortHash(h: string): string {
  if (!h || h.length < 12) return h || "—";
  return `${h.slice(0, 6)}…${h.slice(-6)}`;
}

const COLORS: Record<NodeKind, { fill: string; stroke: string; text: string }> = {
  leaf: {
    fill: "color-mix(in oklch, var(--accent-500) 12%, transparent)",
    stroke: "var(--accent-500)",
    text: "var(--accent-500)",
  },
  path: {
    fill: "var(--surface-2)",
    stroke: "var(--border-strong)",
    text: "var(--ink-muted)",
  },
  root: {
    fill: "color-mix(in oklch, var(--success-500) 12%, transparent)",
    stroke: "var(--success-500)",
    text: "var(--success-500)",
  },
};

// ─── Graph builder ───────────────────────────────────────────────────────────

function buildGraph(proof: ProofResult): {
  nodes: MerkleNode[];
  edges: MerkleEdge[];
  svgW: number;
  svgH: number;
} {
  // Two public signals: leaf → root. The sibling path elements that connect
  // them are private witness data, never exposed to the frontend.
  const levels: { hash: string; kind: NodeKind }[][] = [
    [{ hash: proof.leafHex, kind: "leaf" }],
    [{ hash: proof.rootHex, kind: "root" }],
  ];

  const svgW = levels.length * (NODE_W + H_GAP) + H_GAP;
  const maxRows = Math.max(...levels.map((l) => l.length));
  const svgH = maxRows * (NODE_H + V_GAP) + V_GAP * 2;

  const nodes: MerkleNode[] = [];
  const edges: MerkleEdge[] = [];
  const prevIds: string[] = [];

  levels.forEach((level, col) => {
    const x = H_GAP + col * (NODE_W + H_GAP);
    const levelH = level.length * NODE_H + (level.length - 1) * V_GAP;
    const startY = (svgH - levelH) / 2;

    level.forEach((item, row) => {
      const id = `n-${col}-${row}`;
      const y = startY + row * (NODE_H + V_GAP);
      nodes.push({ id, label: shortHash(item.hash), fullHash: item.hash, kind: item.kind, x, y });

      // Connect from every node in previous column to this node
      if (col > 0) {
        prevIds.forEach((pid) => edges.push({ fromId: pid, toId: id }));
      }
    });

    // Replace prevIds for next iteration
    prevIds.length = 0;
    level.forEach((_, row) => prevIds.push(`n-${col}-${row}`));
  });

  return { nodes, edges, svgW, svgH };
}

// ─── Component ───────────────────────────────────────────────────────────────

export function MerklePathViz({ proof }: { proof: ProofResult }) {
  const { nodes, edges, svgW, svgH } = buildGraph(proof);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const nodeMap = new Map(nodes.map((n) => [n.id, n]));
  const hovered = hoveredId ? nodeMap.get(hoveredId) : null;

  return (
    <div className="merkle-viz">
      <div className="merkle-viz__label">
        Proof — leaf #{proof.leafIndex} → root
      </div>

      <div className="merkle-viz__scroll">
        <svg
          width={svgW}
          height={svgH}
          viewBox={`0 0 ${svgW} ${svgH}`}
          xmlns="http://www.w3.org/2000/svg"
          style={{ display: "block" }}
        >
          {/* Edges */}
          {edges.map(({ fromId, toId }, i) => {
            const a = nodeMap.get(fromId)!;
            const b = nodeMap.get(toId)!;
            const x1 = a.x + NODE_W;
            const y1 = a.y + NODE_H / 2;
            const x2 = b.x;
            const y2 = b.y + NODE_H / 2;
            // Cubic bezier for smooth curve
            const cx = (x1 + x2) / 2;
            return (
              <path
                key={i}
                d={`M ${x1} ${y1} C ${cx} ${y1}, ${cx} ${y2}, ${x2} ${y2}`}
                fill="none"
                stroke="var(--border-strong)"
                strokeWidth="1.5"
                opacity="0.5"
              />
            );
          })}

          {/* Nodes */}
          {nodes.map((node) => {
            const c = COLORS[node.kind];
            const isHovered = hoveredId === node.id;
            return (
              <g
                key={node.id}
                onMouseEnter={() => setHoveredId(node.id)}
                onMouseLeave={() => setHoveredId(null)}
                style={{ cursor: "default" }}
              >
                <rect
                  x={node.x}
                  y={node.y}
                  width={NODE_W}
                  height={NODE_H}
                  rx="6"
                  fill={c.fill}
                  stroke={c.stroke}
                  strokeWidth={isHovered ? "2" : "1.5"}
                />
                <text
                  x={node.x + NODE_W / 2}
                  y={node.y + NODE_H / 2}
                  textAnchor="middle"
                  dominantBaseline="middle"
                  fontSize="10"
                  fontFamily="var(--font-mono, monospace)"
                  fill={c.text}
                >
                  {node.label}
                </text>
              </g>
            );
          })}
        </svg>
      </div>

      {/* Hover tooltip — full hash */}
      {hovered && (
        <div className="merkle-viz__tooltip">
          <span className="merkle-viz__tooltip-kind">{hovered.kind}</span>
          <span className="merkle-viz__tooltip-hash">{hovered.fullHash}</span>
        </div>
      )}

      {/* Legend */}
      <div className="merkle-viz__legend">
        <span className="merkle-viz__legend-item merkle-viz__legend-item--leaf">Leaf (public)</span>
        <span className="merkle-viz__legend-item merkle-viz__legend-item--root">Root (public)</span>
      </div>
    </div>
  );
}
