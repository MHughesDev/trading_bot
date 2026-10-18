/**
 * The traffic form mirrors the server's rule (`allocated + requested <= 100`,
 * per environment) so an allocation the registry would refuse is refused before
 * the request, with the reason, rather than as a generic failure afterwards.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { DeployModal } from '../ModelDeploymentsTab'
import type { ModelDeployment } from '@/api/mlops'

const dep = (traffic: number, environment: string): ModelDeployment => ({
  deployment_id: `d-${traffic}-${environment}`,
  version: 1,
  environment,
  alias: 'production',
  status: 'active',
  traffic_pct: traffic,
  deployed_at: '2026-10-01T00:00:00Z',
})

function setup(deployments: ModelDeployment[]) {
  const onDeploy = vi.fn()
  render(
    <DeployModal
      open
      versions={[3, 2, 1]}
      deployments={deployments}
      busy={false}
      error={null}
      onClose={() => {}}
      onDeploy={onDeploy}
    />,
  )
  return { onDeploy }
}

describe('a traffic split over 100% is not a deployment', () => {
  it('disables Deploy and names the environment when it would exceed 100', () => {
    setup([dep(60, 'paper')])
    // The slider starts at 100, and paper already routes 60.
    expect(screen.getByRole('button', { name: 'Deploy' })).toBeDisabled()
    expect(screen.getByRole('alert')).toHaveTextContent(/paper already routes 60%/)
  })

  it('enables Deploy once the allocation fits and sends it', () => {
    const { onDeploy } = setup([dep(60, 'paper')])
    fireEvent.change(screen.getByLabelText('Traffic percent'), { target: { value: '40' } })
    const btn = screen.getByRole('button', { name: 'Deploy' })
    expect(btn).toBeEnabled()
    fireEvent.click(btn)
    expect(onDeploy).toHaveBeenCalledWith({ version: 3, environment: 'paper', traffic_pct: 40 })
  })

  it('counts allocation per environment, so live is unaffected by paper', () => {
    setup([dep(60, 'paper')])
    fireEvent.click(screen.getByRole('radio', { name: 'Live' }))
    // Switching to live resets the slider to what live has free: all of it.
    expect(screen.getByRole('button', { name: 'Deploy' })).toBeEnabled()
  })
})
