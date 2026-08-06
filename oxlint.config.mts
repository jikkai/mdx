import amamo from '@amamo/oxlint-config'

export default amamo(
  {
    node: true,
    rules: {
      'vitest/expect-expect': 'off',
    },
    test: 'vitest',
  },
  {
    overrides: [
      {
        files: ['src/compiler.ts', 'src/__tests__/vite.test.ts'],
        rules: { 'no-await-in-loop': 'off' },
      },
      {
        files: ['native.cjs'],
        rules: { 'preserve-caught-error': 'off' },
      },
    ],
  },
)
