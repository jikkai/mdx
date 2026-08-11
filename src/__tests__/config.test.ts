import assert from 'node:assert/strict'

import { test } from 'vitest'

import { defineConfig, normalizeConfig } from '../config.js'

test('normalizes plain configuration and rejects executable values', () => {
  const config = defineConfig({
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
  })

  const normalized = normalizeConfig(config)
  const posts = normalized.collections.posts
  assert.ok(posts)
  assert.equal(posts.directory, '/project/content/posts')
  assert.equal(normalized.generatedDirectory, '/project/.amamo-mdx')
  assert.equal(normalized.cache.directory, '/project/.amamo-mdx/cache')
  assert.equal(normalized.mdx.gfm, true)
  assert.equal(normalized.highlight.unknownLanguage, 'error')
  assert.throws(
    () => normalizeConfig({ ...config, derived: { readingTime: (() => true) as never } }),
    /AMAMO_CONFIG_NOT_SERIALIZABLE/,
  )
})

test('rejects cyclic and non-plain configuration', () => {
  const cyclic: Record<string, unknown> = {}
  cyclic.self = cyclic

  assert.throws(() => normalizeConfig(cyclic as never), /AMAMO_CONFIG_NOT_SERIALIZABLE/)
  assert.throws(
    () =>
      normalizeConfig(
        new (class Config {
          value = true
        })() as never,
      ),
    /AMAMO_CONFIG_NOT_SERIALIZABLE/,
  )
})

test('normalizes math configuration', () => {
  const config = defineConfig({
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: { type: 'object' },
      },
    },
  })

  assert.deepEqual(normalizeConfig(config).math, {
    enabled: false,
    macros: {},
    singleDollar: true,
  })
  assert.deepEqual(normalizeConfig({ ...config, math: false }).math, {
    enabled: false,
    macros: {},
    singleDollar: true,
  })
  assert.deepEqual(normalizeConfig({ ...config, math: {} }).math, {
    enabled: true,
    macros: {},
    singleDollar: true,
  })

  const macros = { '\\RR': '\\mathbb{R}' }
  const normalized = normalizeConfig({ ...config, math: { macros, singleDollar: false } })
  assert.deepEqual(normalized.math, {
    enabled: true,
    macros,
    singleDollar: false,
  })
  assert.notEqual(normalized.math.macros, macros)
})

test('rejects math macro names without a leading backslash', () => {
  const config = defineConfig({
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: { type: 'object' },
      },
    },
    math: { macros: { RR: '\\mathbb{R}' } },
  })

  assert.throws(
    () => normalizeConfig(config),
    /AMAMO_CONFIG_INVALID: math\.macros\.RR must start with a backslash/,
  )
})

test('bounds configured math macro bytes', () => {
  const config = defineConfig({
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: { type: 'object' },
      },
    },
  })
  const exactLimit = Object.fromEntries(
    Array.from({ length: 15 }, (_, index) => [
      `\\M${String.fromCharCode('A'.charCodeAt(0) + index)}`,
      'x'.repeat(1024),
    ]),
  )
  const usedBytes = Object.entries(exactLimit).reduce(
    (total, [name, expansion]) => total + name.length + expansion.length,
    0,
  )
  const finalName = '\\final'
  exactLimit[finalName] = 'x'.repeat(16384 - usedBytes - finalName.length)

  assert.deepEqual(
    normalizeConfig({ ...config, math: { macros: exactLimit } }).math.macros,
    exactLimit,
  )
  assert.throws(
    () =>
      normalizeConfig({
        ...config,
        math: { macros: { '\\oversized': 'é'.repeat(513) } },
      }),
    /AMAMO_CONFIG_INVALID: math\.macros\.\\oversized exceeds the 1024-byte limit/,
  )
  assert.throws(
    () =>
      normalizeConfig({
        ...config,
        math: { macros: { ...exactLimit, '\\extra': '' } },
      }),
    /AMAMO_CONFIG_INVALID: math\.macros exceeds the 16384-byte total limit/,
  )
})
