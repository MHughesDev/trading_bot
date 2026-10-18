import { Bot, BarChart2, Brain, Code2, Shield, Zap } from 'lucide-react'
import type { ModelKind } from '@/api/mlops'

export const KIND_ICONS: Record<ModelKind, React.ElementType> = {
  forecaster: Brain,
  signal_ranker: BarChart2,
  trade_decision: Zap,
  risk_sizing: Shield,
  embedding: Code2,
  external_llm_adapter: Bot,
}
