import { Handle, Position, type NodeProps } from '@xyflow/react'
import { NodeShell } from '@/nodes/NodeShell'
import type { Arg } from '@/api/strategyV3'
import { describe, nodeFamily, showArg, type Kind } from './graph'

/* One `tbot.strategy/3` node on the canvas (Set U U-7.5).
   A handle per port that takes a wire (left) and per output (right), each
   labelled with its type: the canvas refuses a wire whose types do not fit
   (`canWire`), and the label says why before anyone tries. Literal arguments
   are shown, and edited in the inspector. */

export interface GraphOpNodeData extends Record<string, unknown> {
  op: string
  family: string
  inputs: { port: string; kind: Kind; optional: boolean }[]
  outputs: { handle: string; kind: Kind }[]
  literals: Record<string, Arg>
  invalid?: string
}

export function GraphOpNode({ id, data, selected }: NodeProps) {
  const d = data as GraphOpNodeData
  const literals = Object.entries(d.literals)
  return (
    <div style={{ position: 'relative', minWidth: 190 }}>
      <NodeShell
        family={nodeFamily(d.family)}
        familyLabel={d.op}
        title={id}
        selected={selected}
        invalid={!!d.invalid}
        warning={d.invalid}
      >
        {literals.map(([k, v]) => (
          <div key={k} className="fld">
            <span className="k">{k}</span>
            <span className="v mono truncate-1">{showArg(v)}</span>
          </div>
        ))}
      </NodeShell>
      {d.inputs.map((p, i) => (
        <Handle
          key={p.port}
          id={p.port}
          type="target"
          position={Position.Left}
          title={`${p.port}: ${describe(p.kind)}${p.optional ? ' (optional)' : ''}`}
          style={{ top: 30 + i * 16 }}
        />
      ))}
      {d.outputs.map((o, i) => (
        <Handle
          key={o.handle}
          id={o.handle}
          type="source"
          position={Position.Right}
          title={`${o.handle}: ${describe(o.kind)}`}
          style={{ top: 30 + i * 16 }}
        />
      ))}
    </div>
  )
}
