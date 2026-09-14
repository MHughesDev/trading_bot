import { cva } from 'class-variance-authority'

/* Legacy shim — see button.variants.ts. Resolves to the `.badge` component
   layer so every badge in the product wears the same tokens. */

export const badgeVariants = cva('badge', {
  variants: {
    variant: {
      default: 'info',
      active: 'pos',
      inactive: 'neutral',
      warning: 'warn',
      destructive: 'neg',
      outline: 'outline',
      accent: 'accent',
    },
  },
  defaultVariants: { variant: 'default' },
})
