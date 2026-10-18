// The Model Studio's route table (N-5.1): one page App.tsx mounts at
// `/mlops/studio/*`, so the studio needs a single lazy import and a single
// <Route> there.
//
//   /mlops/studio             Command Center (N-5.2)
//   /mlops/studio/new         Create wizard (N-5.3)
//   /mlops/studio/:id/*       Cockpit and its tabs (N-5.4 to N-5.9)
import { Navigate, Route, Routes } from 'react-router-dom'
import { ModelCommandCenterPage } from './ModelCommandCenterPage'
import { ModelCreateWizardPage } from './ModelCreateWizardPage'
import { ModelCockpitPage } from './ModelCockpitPage'

export function ModelStudioPage() {
  return (
    <Routes>
      <Route index element={<ModelCommandCenterPage />} />
      <Route path="new" element={<ModelCreateWizardPage />} />
      <Route path=":id/*" element={<ModelCockpitPage />} />
      <Route path="*" element={<Navigate to="/mlops/studio" replace />} />
    </Routes>
  )
}
