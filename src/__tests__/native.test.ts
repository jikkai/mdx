import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'

import { test } from 'vitest'

import type { IAmamoMdxConfig } from '../config.js'

import { normalizeConfig } from '../config.js'
import {
  AmamoMdxError,
  prepareNativeBatch,
  pruneNativeCache,
  renderNativeManifests,
} from '../native.js'
import { createShikiRenderer } from '../shiki.js'

const sourceConfig: IAmamoMdxConfig = {
  cache: false,
  root: '/project',
  collections: {
    posts: {
      directory: 'content/posts',
      schema: {
        $schema: 'https://json-schema.org/draft/2020-12/schema',
        type: 'object',
        properties: {
          title: { type: 'string' },
          draft: { type: 'boolean', default: false },
          password: { type: 'string' },
        },
        required: ['title'],
      },
      sensitive: ['password'],
    },
  },
}
const config = normalizeConfig(sourceConfig)

test('loads the real addon and returns only public frontmatter', () => {
  const batch = prepareNativeBatch(config, [
    {
      collection: 'posts',
      file: '/project/content/posts/hello.mdx',
      key: 'hello',
      source: '---\ntitle: Hello\npassword: secret\n---\n# Hello\n',
    },
  ])

  assert.deepEqual(batch.codeBlocks, [])
  const records = batch.finish([])
  assert.equal(records[0]?.frontmatter.title, 'Hello')
  assert.equal(records[0]?.frontmatter.draft, false)
  assert.equal(records[0]?.frontmatter.password, undefined)
  assert.doesNotMatch(JSON.stringify(records), /secret/)
})

test('maps schema failures to structured diagnostics', () => {
  assert.throws(
    () =>
      prepareNativeBatch(config, [
        {
          collection: 'posts',
          file: '/project/content/posts/bad.mdx',
          key: 'bad',
          source: '---\ndraft: true\n---\n',
        },
      ]),
    (error: unknown) => {
      assert.ok(error instanceof AmamoMdxError)
      assert.equal(error.diagnostics[0]?.code, 'AMAMO_SCHEMA_INVALID')
      return true
    },
  )
})

test('counts CJK text but excludes code from reading time', () => {
  const readingConfig = normalizeConfig({
    cache: false,
    highlight: false,
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: {
          type: 'object',
          properties: { title: { type: 'string' } },
          required: ['title'],
        },
      },
    },
    derived: { readingTime: true },
  })
  const batch = prepareNativeBatch(readingConfig, [
    {
      collection: 'posts',
      file: '/project/content/posts/cjk.mdx',
      key: 'cjk',
      source: `---\ntitle: CJK\n---\n${'字'.repeat(201)}\n\n\`\`\`text\n${'字'.repeat(500)}\n\`\`\`\n`,
    },
  ])

  assert.deepEqual(batch.finish([])[0]?.derived.readingTime, { minutes: 1, words: 201 })
})

test('injects real Shiki HAST into the compiled module', async () => {
  const batch = prepareNativeBatch(config, [
    {
      collection: 'posts',
      file: '/project/content/posts/code.mdx',
      key: 'code',
      source: '---\ntitle: Code\n---\n```ts\nconst n: number = 1\n```\n```text\nplain text\n```\n',
    },
  ])
  assert.equal(batch.codeBlocks.length, 2)
  const renderer = await createShikiRenderer(config.highlight)

  try {
    const records = batch.finish(await renderer.highlight(batch.codeBlocks))
    assert.match(records[0]?.module ?? '', /--shiki-dark/)
    assert.match(records[0]?.module ?? '', /"backgroundColor": "#ffffff"/)
    assert.match(records[0]?.module ?? '', /style: \{/)
    assert.doesNotMatch(records[0]?.module ?? '', /style: "--shiki-dark/)
    assert.match(records[0]?.module ?? '', /language-ts/)
    assert.doesNotMatch(records[0]?.module ?? '', /language-text/)
  } finally {
    renderer.dispose()
  }
})

test('leaves dollar-delimited text unchanged when math is disabled', () => {
  const batch = prepareNativeBatch(config, [
    {
      collection: 'posts',
      file: '/project/content/posts/plain-dollar.mdx',
      key: 'plain-dollar',
      source: '---\ntitle: Plain dollar\n---\nThe value is $x$.\n',
    },
  ])
  const module = batch.finish([])[0]?.module ?? ''

  assert.match(module, /The value is \$x\$/)
  assert.doesNotMatch(module, /amamo-math/)
})

test('changes cache keys when math macros change', () => {
  const input = {
    collection: 'posts',
    file: '/project/content/posts/cache-math.mdx',
    key: 'cache-math',
    source: '---\ntitle: Cache math\n---\n$\\numberSet$\n',
  }
  const natural = normalizeConfig({
    ...sourceConfig,
    highlight: false,
    math: { macros: { '\\numberSet': '\\mathbb{N}' } },
  })
  const real = normalizeConfig({
    ...sourceConfig,
    highlight: false,
    math: { macros: { '\\numberSet': '\\mathbb{R}' } },
  })

  const naturalKey = prepareNativeBatch(natural, [input]).finish([])[0]?.cacheKey
  const realKey = prepareNativeBatch(real, [input]).finish([])[0]?.cacheKey

  assert.notEqual(naturalKey, realKey)
})

test('renders configured math as self-contained SVG without shifting Shiki blocks', async () => {
  const mathConfig = normalizeConfig({
    ...sourceConfig,
    math: { macros: { '\\RR': '\\mathbb{R}' } },
  })
  const batch = prepareNativeBatch(mathConfig, [
    {
      collection: 'posts',
      file: '/project/content/posts/math.mdx',
      key: 'math',
      source:
        '---\ntitle: Math\n---\nThe set $\\RR$ is infinite.\n\n$$\n\\sum_{i=1}^n i\n$$\n\n```ts\nconst n = 1\n```\n',
    },
  ])
  assert.equal(batch.codeBlocks.length, 1)
  assert.equal(batch.codeBlocks[0]?.lang, 'ts')
  const renderer = await createShikiRenderer(mathConfig.highlight)

  try {
    const module = batch.finish(await renderer.highlight(batch.codeBlocks))[0]?.module ?? ''
    assert.match(module, /amamo-math-inline/)
    assert.match(module, /amamo-math-display/)
    assert.match(module, /currentColor/)
    assert.match(module, /verticalAlign/)
    assert.match(module, /role: "math"/)
    assert.match(module, /"aria-label": "\\\\RR"/)
    assert.match(module, /language-ts/)
    assert.doesNotMatch(module, /language-math/)
    assert.doesNotMatch(module, /dangerouslySetInnerHTML|katex|ratex|<text/)
  } finally {
    renderer.dispose()
  }
})

test('reports invalid math through the native diagnostic interface', () => {
  const mathConfig = normalizeConfig({ ...sourceConfig, math: {} })

  assert.throws(
    () =>
      prepareNativeBatch(mathConfig, [
        {
          collection: 'posts',
          file: '/project/content/posts/bad-math.mdx',
          key: 'bad-math',
          source: '---\ntitle: Bad math\n---\n$\\frac{$\n',
        },
      ]),
    (error: unknown) => {
      assert.ok(error instanceof AmamoMdxError)
      assert.equal(error.diagnostics[0]?.code, 'AMAMO_MATH_PARSE')
      assert.equal(error.diagnostics[0]?.file, '/project/content/posts/bad-math.mdx')
      return true
    },
  )
})

test('keeps fenced math code separate from rendered formulas', async () => {
  const mathConfig = normalizeConfig({
    ...sourceConfig,
    highlight: {
      provider: 'shiki',
      themes: { dark: 'vitesse-dark', light: 'vitesse-light' },
      unknownLanguage: 'plain',
    },
    math: {},
  })
  const batch = prepareNativeBatch(mathConfig, [
    {
      collection: 'posts',
      file: '/project/content/posts/math-code.mdx',
      key: 'math-code',
      source: '---\ntitle: Math code\n---\n```math\nx + y\n```\n',
    },
  ])
  assert.equal(batch.codeBlocks.length, 1)
  assert.equal(batch.codeBlocks[0]?.lang, 'math')
  const renderer = await createShikiRenderer(mathConfig.highlight)

  try {
    const module = batch.finish(await renderer.highlight(batch.codeBlocks))[0]?.module ?? ''
    assert.match(module, /language-math/)
    assert.doesNotMatch(module, /amamo-math-(?:inline|display)/)
  } finally {
    renderer.dispose()
  }
})

test('can disable single-dollar inline math without disabling display math', () => {
  const mathConfig = normalizeConfig({ ...sourceConfig, math: { singleDollar: false } })
  const batch = prepareNativeBatch(mathConfig, [
    {
      collection: 'posts',
      file: '/project/content/posts/currency.mdx',
      key: 'currency',
      source: '---\ntitle: Currency\n---\nCosts $5 today.\n\n$$\nx + 1\n$$\n',
    },
  ])
  const module = batch.finish([])[0]?.module ?? ''

  assert.match(module, /Costs \$5 today/)
  assert.match(module, /amamo-math-display/)
  assert.doesNotMatch(module, /amamo-math-inline/)
})

test('persists safe records and recovers a corrupt cache entry', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'amamo-mdx-native-'))
  const cachedConfig = normalizeConfig({
    highlight: false,
    root,
    collections: {
      posts: {
        directory: 'content/posts',
        schema: {
          type: 'object',
          properties: {
            title: { type: 'string' },
            password: { type: 'string' },
          },
          required: ['title'],
        },
        sensitive: ['password'],
      },
    },
    manifests: {
      server: {
        output: 'server.json',
        key: 'title',
        fields: {
          title: 'title',
          passwordHash: { from: 'password', transform: 'sha256' },
          protected: { from: 'password', transform: 'exists' },
        },
      },
    },
  })
  const input = {
    collection: 'posts',
    file: path.join(root, 'content/posts/hello.mdx'),
    key: 'hello',
    source: '---\ntitle: Hello\npassword: secret\n---\n# Hello\n',
  }

  try {
    const first = prepareNativeBatch(cachedConfig, [input]).finish([])
    const record = first[0]
    assert.ok(record)
    const cacheFile = path.join(
      cachedConfig.cache.directory,
      record.cacheKey.slice(0, 2),
      `${record.cacheKey}.json`,
    )
    assert.doesNotMatch(await readFile(cacheFile, 'utf8'), /secret/)

    const rendered = renderNativeManifests(cachedConfig, first)
    assert.equal(rendered.length, 1)
    assert.match(rendered[0]?.contents ?? '', /2bb80d537b1da3e/)
    assert.doesNotMatch(rendered[0]?.contents ?? '', /secret/)

    await writeFile(cacheFile, '{truncated')
    const recovered = prepareNativeBatch(cachedConfig, [input]).finish([])
    assert.equal(recovered[0]?.diagnostics[0]?.code, 'AMAMO_CACHE_CORRUPT')
    assert.doesNotMatch(await readFile(cacheFile, 'utf8'), /secret/)
    assert.equal(pruneNativeCache(cachedConfig.cache.directory, []), 1)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
