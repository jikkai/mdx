import assert from 'node:assert/strict'

import { test } from 'vitest'

import { createShikiRenderer } from '../shiki.js'

const config = {
  colorReplacements: { '#ffffff': '#f0efea' },
  enabled: true,
  engine: 'oniguruma',
  languages: 'auto',
  provider: 'shiki',
  themes: { light: 'vitesse-light', dark: 'vitesse-dark' },
  unknownLanguage: 'error',
} as const

test('uses official Shiki dual-theme HAST', async () => {
  const renderer = await createShikiRenderer(config)
  try {
    const result = await renderer.highlight([
      {
        blockId: 0,
        code: 'const n: number = 1',
        documentId: 'posts/a',
        lang: 'ts',
        meta: null,
      },
    ])

    assert.equal(result.length, 1)
    const hast = JSON.stringify(result[0]?.hast)
    assert.match(hast, /background-color:#f0efea/)
    assert.match(hast, /--shiki-dark/)
  } finally {
    await renderer.dispose()
  }
})

test('highlights embedded JSX in MDX code', async () => {
  const renderer = await createShikiRenderer(config)
  try {
    const result = await renderer.highlight([
      {
        blockId: 0,
        code: '<Callout variant="tip">Rendered during the build.</Callout>',
        documentId: 'posts/a',
        lang: 'mdx',
        meta: null,
      },
    ])
    const hast = JSON.stringify(result[0]?.hast)
    const tokenColors = new Set(
      [...hast.matchAll(/"style":"color:([^;"]+)/g)].map((match) => match[1]),
    )

    assert.ok(tokenColors.size > 1)
  } finally {
    renderer.dispose()
  }
})

test('applies the documentation code annotation semantics', async () => {
  const renderer = await createShikiRenderer(config)
  try {
    const result = await renderer.highlight([
      {
        blockId: 0,
        code: [
          'const target = true // [!code ++]',
          'const focus = true // [!code focus]',
          'const broken = true // [!code error]',
          'const marked = true // [!code word:marked]',
        ].join('\n'),
        documentId: 'posts/a',
        lang: 'ts',
        meta: '{1} /target/ filename="doctrine.config.ts"',
      },
    ])
    const hast = JSON.stringify(result[0]?.hast)

    assert.match(hast, /language-ts/)
    assert.doesNotMatch(hast, /shiki,shiki-themes/)
    assert.match(hast, /line-highlighted/)
    assert.match(hast, /line-diff-add/)
    assert.match(hast, /line-focused/)
    assert.match(hast, /line-error/)
    assert.match(hast, /word-highlighted/)
    assert.match(hast, /has-word-highlighted/)
    assert.match(hast, /data-meta.*filename.*doctrine\.config\.ts/)
  } finally {
    renderer.dispose()
  }
})

test('unknown language behavior is configurable', async () => {
  const strict = await createShikiRenderer(config)
  await assert.rejects(
    strict.highlight([
      {
        blockId: 0,
        code: 'plain',
        documentId: 'posts/a',
        lang: 'definitely-not-a-language',
        meta: null,
      },
    ]),
    /AMAMO_SHIKI_UNKNOWN_LANGUAGE/,
  )
  await strict.dispose()

  const plain = await createShikiRenderer({ ...config, unknownLanguage: 'plain' })
  try {
    const result = await plain.highlight([
      {
        blockId: 0,
        code: 'plain',
        documentId: 'posts/a',
        lang: 'definitely-not-a-language',
        meta: null,
      },
    ])
    assert.match(JSON.stringify(result[0]?.hast), /plain/)
  } finally {
    await plain.dispose()
  }
})

test('expands duplicate highlights back to their document identities', async () => {
  const renderer = await createShikiRenderer(config)
  try {
    const result = await renderer.highlight([
      {
        blockId: 1,
        code: 'const duplicate = true',
        documentId: 'posts/a',
        lang: 'ts',
        meta: null,
      },
      {
        blockId: 4,
        code: 'const duplicate = true',
        documentId: 'posts/b',
        lang: 'ts',
        meta: null,
      },
    ])

    assert.deepEqual(
      result.map(({ blockId, documentId }) => ({ blockId, documentId })),
      [
        { blockId: 1, documentId: 'posts/a' },
        { blockId: 4, documentId: 'posts/b' },
      ],
    )
    assert.deepEqual(result[0]?.hast, result[1]?.hast)
  } finally {
    await renderer.dispose()
  }
})
