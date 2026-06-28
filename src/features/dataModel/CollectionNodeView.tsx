import { Handle, Position } from "@xyflow/react";
import type { CollectionShape } from "../../ipc/commands";

export interface CollectionNodeData {
  shape: CollectionShape;
}

const VISIBLE_FIELDS = 8;

export function CollectionNode({ data }: { data: CollectionNodeData }) {
  const { shape } = data;
  const fields = shape.root.children.slice(0, VISIBLE_FIELDS);
  const hasMore = shape.root.children.length > VISIBLE_FIELDS;

  return (
    <div className="collection-node">
      <Handle
        type="target"
        position={Position.Left}
        id="_id"
        className="collection-node__handle collection-node__handle--target"
      />
      <div className="collection-node__header">
        <span className="collection-node__name" title={shape.collection}>
          {shape.collection}
        </span>
        <span className="collection-node__count">
          {shape.documentCount != null ? shape.documentCount.toLocaleString() : "?"}
        </span>
      </div>
      <div className="collection-node__fields">
        {fields.map((field) => {
          const dominant = Object.entries(field.types).sort((a, b) => b[1] - a[1])[0];
          const isRef = dominant?.[0] === "objectId";
          const isArray = field.types["array"] != null && field.types["array"] > 0;
          return (
            <div key={field.path} className={`collection-node__field${isRef ? " collection-node__field--ref" : ""}`}>
              <span className="collection-node__field-name" title={field.path}>
                {field.name}
              </span>
              <span className="collection-node__field-type">
                {isArray ? "[]" : ""}{dominant?.[0] ?? "unknown"}
              </span>
              {isRef && (
                <Handle
                  type="source"
                  position={Position.Right}
                  id={field.path}
                  className="collection-node__handle collection-node__handle--source"
                />
              )}
            </div>
          );
        })}
        {hasMore && (
          <div className="collection-node__more">
            +{shape.root.children.length - VISIBLE_FIELDS} more
          </div>
        )}
      </div>
    </div>
  );
}
