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
