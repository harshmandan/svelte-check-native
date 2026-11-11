import type Generic from './Generic.svelte'

type InvalidKeyProps = Parameters<typeof Generic<{ key: 'entter' }>>[1]
export const keyIsNever: [InvalidKeyProps['key']] extends [never] ? true : false = true

type ValidKeyProps = Parameters<typeof Generic<{ key: 'enter' }>>[1]
export const validKey: ValidKeyProps['key'] = 'enter'
