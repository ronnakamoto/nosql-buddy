import { useEffect, useState } from "react";
import commands, { type ShapeNode } from "../ipc/commands";

export interface SchemaInfo {
  /** Flat list of top-level field names. */
  topLevelFields: string[];
  /** All nested field paths (dot-notation). */
  allPaths: string[];
  /** Paths grouped by parent prefix (e.g. "address" -> ["address.street", "address.city"]). */
  childrenByPrefix: Map<string, string[]>;
  /** Whether the schema is currently loading. */
  loading: boolean;
  /** Error message if loading failed. */
  error: string | null;
}

function flattenShapeNodes(nodes: ShapeNode[]): { topLevel: string[]; all: string[]; children: Map<string, string[]> } {
  const topLevel: string[] = [];
  const all: string[] = [];
  const children = new Map<string, string[]>();

  function walk(nodeList: ShapeNode[], prefix: string) {
    for (const node of nodeList) {
      const path = prefix ? `${prefix}.${node.name}` : node.name;
      if (!prefix) topLevel.push(node.name);
      all.push(path);

      const childPaths: string[] = [];
      if (node.children) {
        for (const child of node.children) {
          const childPath = `${path}.${child.name}`;
          childPaths.push(childPath);
        }
      }
      if (node.arrayItem?.children) {
        for (const child of node.arrayItem.children) {
          const childPath = `${path}.${child.name}`;
          if (!childPaths.includes(childPath)) childPaths.push(childPath);
        }
      }
      children.set(path, childPaths);

      if (node.children) walk(node.children, path);
      if (node.arrayItem?.children) walk(node.arrayItem.children, path);
    }
  }

  walk(nodes, "");
  return { topLevel, all, children };
}

/**
 * Fetches the collection shape via `sample_shape` and returns a rich
 * schema descriptor suitable for autocomplete. The shape is sampled
 * once per mount / connection+db+collection change and cached in
 * component state so suggestion lists are instant.
 */
export function useCollectionSchema(
  connectionId: string,
  database: string,
  collection: string,
): SchemaInfo {
  const [info, setInfo] = useState<SchemaInfo>({
    topLevelFields: [],
    allPaths: [],
    childrenByPrefix: new Map(),
    loading: true,
    error: null,
  });

  useEffect(() => {
    let cancelled = false;
    setInfo((prev) => ({ ...prev, loading: true, error: null }));

    commands
      .sampleShape(connectionId, database, collection, 200)
      .then((shape) => {
        if (cancelled) return;
        const nodes = shape.root?.children ?? [];
        const { topLevel, all, children } = flattenShapeNodes(nodes);
        setInfo({
          topLevelFields: topLevel,
          allPaths: all,
          childrenByPrefix: children,
          loading: false,
          error: null,
        });
      })
      .catch((e) => {
        if (cancelled) return;
        setInfo({
          topLevelFields: [],
          allPaths: [],
          childrenByPrefix: new Map(),
          loading: false,
          error: String(e),
        });
      });

    return () => {
      cancelled = true;
    };
  }, [connectionId, database, collection]);

  return info;
}
