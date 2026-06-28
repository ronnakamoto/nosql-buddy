import { useCallback, useEffect, useMemo, useRef } from "react";
import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  useEdgesState,
  useNodesState,
  useReactFlow,
  type Edge,
  type Node,
  type OnNodeDrag,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { DataModelGraph, RelationshipEdge } from "../../ipc/commands";
import { CollectionNode } from "./CollectionNodeView";
import { RelationshipEdgeView } from "./RelationshipEdgeView";
import { layoutGraph } from "./layoutGraph";
import { InfoPopover } from "../../components/InfoPopover";

const nodeTypes = { collection: CollectionNode };
const edgeTypes = { relationship: RelationshipEdgeView };

/**
 * Per-database session cache of node positions the user has dragged. Keeps
 * manual layouts across rescans and tab switches within one app run (the plan's
 * "manual drag persistence (session)"). Keyed by database so two diagrams don't
 * collide. Cleared only by an app reload.
 */
const positionCache = new Map<string, Map<string, { x: number; y: number }>>();

function savedPositions(db: string): Map<string, { x: number; y: number }> {
  let m = positionCache.get(db);
  if (!m) {
    m = new Map();
    positionCache.set(db, m);
  }
  return m;
}

export interface DiagramCanvasProps {
  graph: DataModelGraph;
  onNodeClick?: (collection: string) => void;
  /** Reports the current laid-out (and user-dragged) nodes + edges whenever
   * they change, so the parent can export the visible layout. */
  onLayoutChange?: (nodes: Node[], edges: Edge[]) => void;
}

export function DiagramCanvas(props: DiagramCanvasProps) {
  return (
    <ReactFlowProvider>
      <DiagramCanvasInner {...props} />
    </ReactFlowProvider>
  );
}

function DiagramCanvasInner({ graph, onNodeClick, onLayoutChange }: DiagramCanvasProps) {
  const initialNodes = useMemo(() => buildNodes(graph), [graph]);
  const initialEdges = useMemo(() => buildEdges(graph.edges), [graph.edges]);

  const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);
  const { fitView } = useReactFlow();
  const onLayoutChangeRef = useRef(onLayoutChange);
  onLayoutChangeRef.current = onLayoutChange;

  const reportLayout = useCallback(
    (ns: Node[], es: Edge[]) => onLayoutChangeRef.current?.(ns, es),
    [],
  );

  // Lay out (or restore) positions whenever the graph data changes. Reuses
  // saved drag positions for this database when present; otherwise runs dagre.
  // Fits once per graph change (not on every drag, which would fight the user).
  useEffect(() => {
    const cached = savedPositions(graph.database);
    const laidOut = layoutGraph(initialNodes, initialEdges);
    if (cached.size > 0) {
      const restored = laidOut.nodes.map((n) => {
        const p = cached.get(n.id);
        return p ? { ...n, position: p } : n;
      });
      setNodes(restored);
    } else {
      setNodes(laidOut.nodes);
    }
    setEdges(laidOut.edges);
    reportLayout(laidOut.nodes, laidOut.edges);
    const timer = setTimeout(() => fitView({ padding: 0.15, duration: 250 }), 50);
    return () => clearTimeout(timer);
  }, [initialNodes, initialEdges, graph.database, setNodes, setEdges, reportLayout, fitView]);

  // Keep the parent informed of the current node positions after each change so
  // exports reflect manual drags, not just the last dagre pass.
  useEffect(() => {
    reportLayout(nodes, edges);
  }, [nodes, edges, reportLayout]);

  const relayout = useCallback(() => {
    // Clear saved positions for this db so dagre re-runs from scratch, then fit.
    savedPositions(graph.database).clear();
    const laidOut = layoutGraph(nodes, edges);
    setNodes(laidOut.nodes);
    setEdges(laidOut.edges);
    setTimeout(() => fitView({ padding: 0.15, duration: 250 }), 0);
  }, [graph.database, nodes, edges, setNodes, setEdges, fitView]);

  const fit = useCallback(() => {
    fitView({ padding: 0.15, duration: 250 });
  }, [fitView]);

  // Keyboard shortcuts: F = fit, R = re-layout. Only when no text input is
  // focused (so typing in a field never triggers a canvas action).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = document.activeElement;
      if (
        el &&
        (el.tagName === "INPUT" ||
          el.tagName === "TEXTAREA" ||
          el.tagName === "SELECT" ||
          (el as HTMLElement).isContentEditable)
      ) {
        return;
      }
      if (e.key === "f" || e.key === "F") {
        e.preventDefault();
        fit();
      } else if (e.key === "r" || e.key === "R") {
        e.preventDefault();
        relayout();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fit, relayout]);

  const onNodeDragStop: OnNodeDrag<Node> = useCallback(
    (_, node) => {
      // Persist the dragged position for this database so it survives rescans.
      savedPositions(graph.database).set(node.id, { ...node.position });
    },
    [graph.database],
  );

  return (
    <div className="diagram-canvas">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        fitView={false}
        proOptions={{ hideAttribution: true }}
        minZoom={0.2}
        maxZoom={2}
        defaultViewport={{ zoom: 0.8, x: 0, y: 0 }}
        onNodeClick={(_, node) => onNodeClick?.(node.id)}
        onNodeDragStop={onNodeDragStop}
      >
        <Background color="var(--border-strong)" gap={16} size={1} />
        <Controls />
        <MiniMap
          nodeColor={(n) => (n.selected ? "var(--accent-500)" : "var(--surface-2)")}
          maskColor="color-mix(in oklch, var(--bg) 60%, transparent)"
          className="diagram-canvas__minimap"
        />
      </ReactFlow>
      <div className="diagram-canvas__shortcuts">
        <span aria-hidden="true">
          <kbd>F</kbd> fit · <kbd>R</kbd> re-layout
        </span>
        <InfoPopover label="Canvas keyboard shortcuts" title="Diagram shortcuts">
          <p>
            <strong>F</strong> — Fit the entire diagram into view.
          </p>
          <p>
            <strong>R</strong> — Re-run auto-layout and reset any manually dragged positions.
          </p>
          <p className="diagram-canvas__shortcuts-note">
            Shortcuts only work when no text field is focused.
          </p>
        </InfoPopover>
      </div>
    </div>
  );
}

function buildNodes(graph: DataModelGraph): Node[] {
  return graph.nodes.map((shape) => ({
    id: shape.collection,
    type: "collection",
    position: { x: 0, y: 0 },
    data: { shape },
  }));
}

function buildEdges(edges: RelationshipEdge[]): Edge[] {
  return edges.map((e) => ({
    id: e.id,
    source: e.fromCollection,
    target: e.toCollection,
    sourceHandle: e.fromField,
    targetHandle: e.toField,
    type: "relationship",
    data: { edge: e },
  }));
}
