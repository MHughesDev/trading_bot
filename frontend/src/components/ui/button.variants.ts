import { cva } from 'class-variance-authority'

/* Legacy shim.
   The Meridian primitives live in `@/components/primitives/Button`. This file
   stays so existing call sites keep working, but every variant now resolves to
   the same `.btn` component layer — no raw palette class, no literal colour.
   Do not add variants here; migrate the call site instead. */

export const buttonVariants = cva('btn', {
  variants: {
    variant: {
      default: 'primary',
      destructive: 'danger',
      outline: '',
      ghost: 'ghost',
      secondary: '',
      success: 'buy',
      warning: '',
      link: 'ghost acc',
    },
    size: {
      default: '',
      sm: 'sm',
      lg: 'lg',
      icon: 'sm',
      'icon-sm': 'xs',
    },
  },
  defaultVariants: { variant: 'default', size: 'default' },
})
