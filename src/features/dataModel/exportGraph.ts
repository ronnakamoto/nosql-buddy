import type { Edge, Node } from "@xyflow/react";
import type {
  CollectionShape,
  DataModelGraph,
  RelationshipEdge,
  RelationshipKind,
  ShapeNode,
  ShapeType,
} from "../../ipc/commands";
import {
  FIELD_HEIGHT,
  HEADER_HEIGHT,
  NODE_WIDTH,
  VISIBLE_FIELDS,
  nodeHeight,
} from "./layoutGraph";

// ─── JSON ───────────────────────────────────────────────────────────

/** Serialize the full inferred model to pretty-printed JSON. */
export function graphToJson(graph: DataModelGraph): string {
  return JSON.stringify(graph, null, 2);
}

// ─── Mermaid erDiagram ──────────────────────────────────────────────

const MERMAID_CARDINALITY: Record<RelationshipKind, string> = {
  "one-to-one": "||--||",
  "one-to-many": "||--o{",
  "many-to-one": "}o--||",
  "many-to-many": "}o--o{",
};

/** Mermaid entity/attribute names reject most punctuation. Collapse anything
 * outside [A-Za-z0-9_] to `_` so collection/field names render safely. */
function mermaidName(name: string): string {
  const cleaned = name.replace(/[^A-Za-z0-9_]/g, "_");
  return cleaned.replace(/^(\d)/, "_$1");
}

function dominantType(field: ShapeNode): string {
  let best: string = "unknown";
  let bestW = -1;
  for (const [t, w] of Object.entries(field.types)) {
    if (w > bestW) {
      best = t;
      bestW = w;
    }
  }
  return best;
}

/** Serialize the model to a Mermaid `erDiagram` block. Position-independent —
 * derived purely from the inferred shapes and edges. */
export function graphToMermaid(graph: DataModelGraph): string {
  const lines: string[] = ["erDiagram"];
  const seen = new Set<string>();

  for (const shape of graph.nodes) {
    const ent = mermaidName(shape.collection);
    if (seen.has(ent)) continue;
    seen.add(ent);
    lines.push(`  ${ent} {`);
    for (const field of shape.root.children.slice(0, 12)) {
      const t = mermaidType(dominantType(field));
      lines.push(`    ${t} ${mermaidName(field.name)}`);
    }
    if (shape.root.children.length > 12) {
      lines.push(`    string ${mermaidName(`__more_${shape.root.children.length - 12}_fields`)}`);
    }
    lines.push("  }");
  }

  for (const e of graph.edges) {
    const card = MERMAID_CARDINALITY[e.kind] ?? "||--o{";
    const from = mermaidName(e.fromCollection);
    const to = mermaidName(e.toCollection);
    const label = mermaidName(e.fromField);
    lines.push(`  ${from} ${card} ${to} : "${label}"`);
  }

  return lines.join("\n");
}

function mermaidType(t: string): string {
  // Mermaid attribute types are free-form labels; keep them short and ER-ish.
  switch (t as ShapeType) {
    case "objectId":
      return "ObjectId";
    case "int":
      return "Int";
    case "long":
      return "Long";
    case "double":
      return "Double";
    case "decimal":
      return "Decimal";
    case "bool":
      return "Boolean";
    case "date":
      return "Date";
    case "object":
      return "Object";
    case "array":
      return "Array";
    case "binary":
      return "Binary";
    case "timestamp":
      return "Timestamp";
    case "null":
      return "Null";
    case "string":
      return "String";
    default:
      return "Unknown";
  }
}

// ─── SVG ────────────────────────────────────────────────────────────

interface SvgPalette {
  bg: string;
  surface: string;
  surface2: string;
  border: string;
  borderStrong: string;
  ink: string;
  inkMuted: string;
  inkFaint: string;
  accent: string;
  accent700: string;
  warning: string;
}

const TOKEN_MAP: Record<keyof SvgPalette, string> = {
  bg: "--bg",
  surface: "--surface",
  surface2: "--surface-2",
  border: "--border",
  borderStrong: "--border-strong",
  ink: "--ink",
  inkMuted: "--ink-muted",
  inkFaint: "--ink-faint",
  accent: "--accent-500",
  accent700: "--accent-700",
  warning: "--warning-500",
};

/**
 * Resolve a CSS custom property to a concrete `rgb()`/`#hex` string by reading
 * the used value off a scratch element. `getComputedStyle(el).getPropertyValue`
 * on a custom prop returns the raw token (e.g. `oklch(...)`), which SVG fill
 * attributes don't always honor, so we set the token as a real color/background
 * and read the resolved value back.
 */
function resolveCssColor(token: string): string {
  if (typeof document === "undefined") return "rgb(0,0,0)";
  const scratch = document.createElement("div");
  scratch.style.display = "none";
  scratch.style.color = `var(${token})`;
  document.body.appendChild(scratch);
  const resolved = getComputedStyle(scratch).color;
  document.body.removeChild(scratch);
  return resolved || "rgb(0,0,0)";
}

function resolvePalette(): SvgPalette {
  const out = {} as SvgPalette;
  (Object.keys(TOKEN_MAP) as (keyof SvgPalette)[]).forEach((k) => {
    out[k] = resolveCssColor(TOKEN_MAP[k]);
  });
  return out;
}

function esc(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Orthogonal "smooth-step" path between two anchor points, source on the right
 * edge of the source node and target on the left edge of the target node. */
function smoothStepPath(
  sx: number,
  sy: number,
  tx: number,
  ty: number,
  radius = 10,
): string {
  const midX = sx + (tx - sx) / 2;
  const c1x = Math.min(sx + radius, midX);
  const c2x = Math.max(tx - radius, midX);
  return [
    `M ${sx} ${sy}`,
    `L ${c1x} ${sy}`,
    `Q ${midX} ${sy} ${midX} ${sy + (ty > sy ? radius : -radius)}`,
    `L ${midX} ${ty - (ty > sy ? radius : -radius)}`,
    `Q ${midX} ${ty} ${c2x} ${ty}`,
    `L ${tx} ${ty}`,
  ].join(" ");
}

function edgeStroke(confidence: number, p: SvgPalette): { stroke: string; width: number; dash: string } {
  if (confidence >= 0.75) return { stroke: p.accent, width: 2, dash: "" };
  if (confidence >= 0.4) return { stroke: p.inkMuted, width: 1, dash: "6,4" };
  return { stroke: p.inkFaint, width: 1, dash: "2,4" };
}

function kindLabel(kind: RelationshipKind): string {
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

export interface SvgExportResult {
  svg: string;
  width: number;
  height: number;
}

/**
 * Render the laid-out graph to a standalone SVG string. Nodes are drawn at the
 * exact positions and dimensions React Flow uses (so a manual drag is captured
 * when the caller passes the current `Node[]`), and edges are drawn as
 * orthogonal smooth-step paths between the source field's right handle and the
 * target node's left center. All colors are resolved from the active theme
 * tokens so the export matches what the user sees.
 */
export function graphToSvg(
  nodes: Node[],
  edges: Edge[],
  database: string,
): SvgExportResult {
  const palette = resolvePalette();
  const pad = 40;

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  const heights = new Map<string, number>();
  for (const n of nodes) {
    const h = nodeHeight(n);
    heights.set(n.id, h);
    minX = Math.min(minX, n.position.x);
    minY = Math.min(minY, n.position.y);
    maxX = Math.max(maxX, n.position.x + NODE_WIDTH);
    maxY = Math.max(maxY, n.position.y + h);
  }
  if (!Number.isFinite(minX)) {
    minX = 0;
    minY = 0;
    maxX = 0;
    maxY = 0;
  }

  const width = Math.ceil(maxX - minX + pad * 2);
  const height = Math.ceil(maxY - minY + pad * 2);
  const ox = -minX + pad;
  const oy = -minY + pad;

  const parts: string[] = [];
  parts.push(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" font-family="IBM Plex Mono, ui-monospace, monospace">`,
  );
  parts.push(`<rect width="${width}" height="${height}" fill="${palette.bg}"/>`);
  parts.push(`<text x="${pad}" y="${pad - 14}" fill="${palette.inkMuted}" font-size="12">Data Model — ${esc(database)}</text>`);

  // Edges first so node bodies paint over the endpoints.
  for (const e of edges) {
    const src = nodes.find((n) => n.id === e.source);
    const tgt = nodes.find((n) => n.id === e.target);
    if (!src || !tgt) continue;
    const rel = (e.data as { edge?: RelationshipEdge } | undefined)?.edge;
    if (!rel) continue;

    const tH = heights.get(tgt.id) ?? nodeHeight(tgt);
    const shape = (src.data as { shape?: CollectionShape } | undefined)?.shape;
    const fields = shape?.root.children ?? [];
    const rowIndex = Math.max(
      0,
      fields.findIndex((f) => f.path === e.sourceHandle),
    );
    const visibleRow = Math.min(rowIndex, VISIBLE_FIELDS - 1);
    const sx = src.position.x + ox + NODE_WIDTH;
    const sy = src.position.y + oy + HEADER_HEIGHT + visibleRow * FIELD_HEIGHT + FIELD_HEIGHT / 2;
    const tx = tgt.position.x + ox;
    const ty = tgt.position.y + oy + tH / 2;

    const { stroke, width: sw, dash } = edgeStroke(rel.confidence, palette);
    parts.push(
      `<path d="${smoothStepPath(sx, sy, tx, ty)}" fill="none" stroke="${stroke}" stroke-width="${sw}"${dash ? ` stroke-dasharray="${dash}"` : ""}/>`,
    );
    const midX = sx + (tx - sx) / 2;
    const labelY = (sy + ty) / 2;
    const label = kindLabel(rel.kind);
    const lw = label.length * 6 + 10;
    parts.push(
      `<rect x="${midX - lw / 2}" y="${labelY - 8}" width="${lw}" height="16" rx="4" fill="${palette.surface}" stroke="${palette.border}" stroke-width="1"/>`,
      `<text x="${midX}" y="${labelY + 3}" text-anchor="middle" fill="${palette.inkMuted}" font-size="10" font-weight="600">${esc(label)}</text>`,
    );
  }

  // Nodes.
  for (const n of nodes) {
    const shape = (n.data as { shape?: CollectionShape } | undefined)?.shape;
    if (!shape) continue;
    const h = heights.get(n.id) ?? nodeHeight(n);
    const x = n.position.x + ox;
    const y = n.position.y + oy;
    const fields = shape.root.children.slice(0, VISIBLE_FIELDS);
    const hasMore = shape.root.children.length > VISIBLE_FIELDS;

    parts.push(
      `<rect x="${x}" y="${y}" width="${NODE_WIDTH}" height="${h}" rx="10" ry="10" fill="${palette.surface}" stroke="${palette.border}" stroke-width="1"/>`,
    );
    parts.push(
      `<rect x="${x}" y="${y}" width="${NODE_WIDTH}" height="${HEADER_HEIGHT}" rx="10" ry="10" fill="${palette.surface2}"/>`,
      `<rect x="${x}" y="${y + HEADER_HEIGHT - 1}" width="${NODE_WIDTH}" height="1" fill="${palette.border}"/>`,
      `<text x="${x + 12}" y="${y + 24}" fill="${palette.ink}" font-size="13" font-weight="600">${esc(shape.collection)}</text>`,
      `<text x="${x + NODE_WIDTH - 12}" y="${y + 24}" text-anchor="end" fill="${palette.inkMuted}" font-size="11">${shape.documentCount != null ? shape.documentCount.toLocaleString() : "?"}</text>`,
    );

    fields.forEach((field, i) => {
      const fy = y + HEADER_HEIGHT + i * FIELD_HEIGHT;
      const dom = dominantType(field);
      const isRef = dom === "objectId";
      const isArray = (field.types["array"] ?? 0) > 0;
      if (isRef) {
        parts.push(
          `<rect x="${x}" y="${fy}" width="${NODE_WIDTH}" height="${FIELD_HEIGHT}" fill="${palette.accent}" fill-opacity="0.06"/>`,
        );
      }
      const nameFill = isRef ? palette.accent700 : palette.ink;
      const typeStr = `${isArray ? "[]" : ""}${dom}`;
      parts.push(
        `<text x="${x + 12}" y="${fy + 17}" fill="${nameFill}" font-size="12">${esc(field.name)}</text>`,
        `<text x="${x + NODE_WIDTH - 12}" y="${fy + 17}" text-anchor="end" fill="${palette.inkMuted}" font-size="11">${esc(typeStr)}</text>`,
      );
      if (isRef) {
        parts.push(
          `<circle cx="${x + NODE_WIDTH + 4}" cy="${fy + FIELD_HEIGHT / 2}" r="4" fill="${palette.accent}" stroke="${palette.accent}" stroke-width="1"/>`,
        );
      }
    });

    if (hasMore) {
      const fy = y + HEADER_HEIGHT + fields.length * FIELD_HEIGHT;
      parts.push(
        `<text x="${x + 12}" y="${fy + 15}" fill="${palette.inkFaint}" font-size="11" font-style="italic">+${shape.root.children.length - VISIBLE_FIELDS} more</text>`,
      );
    }
  }

  parts.push("</svg>");
  return { svg: parts.join("\n"), width, height };
}

// ─── PNG (rasterize the SVG via canvas) ─────────────────────────────

/** Rasterize an SVG string to PNG bytes by drawing it onto a canvas. Uses a 2x
 * scale for crisp output. Standard browser/webview APIs only — no dependency. */
export async function svgToPngBytes(svg: string, width: number, height: number): Promise<Uint8Array> {
  const blob = new Blob([svg], { type: "image/svg+xml;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  try {
    const img = new Image();
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("Failed to render diagram SVG"));
      img.src = url;
    });
    const scale = 2;
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.ceil(width * scale));
    canvas.height = Math.max(1, Math.ceil(height * scale));
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Canvas 2D context unavailable");
    ctx.scale(scale, scale);
    ctx.drawImage(img, 0, 0, width, height);
    const pngBlob = await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("toBlob returned null"))), "image/png");
    });
    return new Uint8Array(await pngBlob.arrayBuffer());
  } finally {
    URL.revokeObjectURL(url);
  }
}
