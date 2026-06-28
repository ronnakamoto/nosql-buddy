import { useEffect, useMemo } from "react";
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
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { DataModelGraph, RelationshipEdge } from "../../ipc/commands";
import { CollectionNode } from "./CollectionNodeView";
import { RelationshipEdgeView } from "./RelationshipEdgeView";
import { layoutGraph } from "./layoutGraph";

const nodeTypes = { collection: CollectionNode };
const edgeTypes = { relationship: RelationshipEdgeView };

export interface DiagramCanvasProps {
  graph: DataModelGraph;
  onNodeClick?: (collection: string) => void;
}

export function DiagramCanvas(props: DiagramCanvasProps) {
  return (
    <ReactFlowProvider>
      <DiagramCanvasInner {...props} />
    </ReactFlowProvider>
  );
}

function DiagramCanvasInner({ graph, onNodeClick }: DiagramCanvasProps) {
  const initialNodes = useMemo(() => buildNodes(graph), [graph]);
  const initialEdges = useMemo(() => buildEdges(graph.edges), [graph.edges]);

  const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);
  const { fitView } = useReactFlow();

  useEffect(() => {
    const laidOut = layoutGraph(initialNodes, initialEdges);
    setNodes(laidOut.nodes);
    setEdges(laidOut.edges);
  }, [initialNodes, initialEdges, setNodes, setEdges]);

  useEffect(() => {
    const timer = setTimeout(() => {
      fitView({ padding: 0.15, duration: 250 });
    }, 50);
    return () => clearTimeout(timer);
  }, [fitView, nodes, edges]);

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
      >
        <Background color="var(--border-strong)" gap={16} size={1} />
        <Controls />
        <MiniMap
          nodeColor={(n) => (n.selected ? "var(--accent-500)" : "var(--surface-2)")}
          maskColor="color-mix(in oklch, var(--bg) 60%, transparent)"
          className="diagram-canvas__minimap"
        />
      </ReactFlow>
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
