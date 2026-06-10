// L-10: badgeVariants is defined in badge.variants.ts and re-exported here
// only as a type convenience. React Fast Refresh requires component files to
// export only React components; non-component exports disable HMR.
import * as React from 'react'
import { type VariantProps } from 'class-variance-authority'
import { cn } from '@/lib/utils'
import { badgeVariants } from './badge.variants'

export { badgeVariants } from './badge.variants'

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />
}

export { Badge }
