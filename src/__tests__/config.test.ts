import assert from 'node:assert/strict'

import { test } from 'vitest'

import type { IFrontmatterSchema } from '../config.js'
import { defineConfig, normalizeConfig, z } from '../config.js'

const emptySchema = z.object({})

test('normalizes Zod configuration and rejects executable values', () => {
  const config = defineConfig({
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: z.object({ title: z.string() }),
      },
    },
  })

  const normalized = normalizeConfig(config)
  const posts = normalized.collections.posts
  assert.ok(posts)
  assert.deepEqual(posts.schema, {
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    type: 'object',
    properties: { title: { type: 'string' } },
    required: ['title'],
    additionalProperties: false,
  })
  assert.equal(posts.directory, '/project/content/posts')
  assert.equal(normalized.generatedDirectory, '/project/.amamo-mdx')
  assert.equal(normalized.cache.directory, '/project/.amamo-mdx/cache')
  assert.equal(normalized.mdx.gfm, true)
  assert.deepEqual(normalized.mdx.extensions, {
    footnotes: true,
    headingIds: false,
    taskLists: true,
  })
  assert.equal(normalized.highlight.unknownLanguage, 'error')
  assert.throws(
    () => normalizeConfig({ ...config, derived: { readingTime: (() => true) as never } }),
    /AMAMO_CONFIG_NOT_SERIALIZABLE/,
  )
  assert.throws(
    () =>
      normalizeConfig({
        ...config,
        collections: {
          posts: { ...config.collections.posts, schema: { type: 'object' } },
        },
      } as never),
    /AMAMO_CONFIG_INVALID: collections\.posts\.schema must be a compatible object schema/,
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

test('accepts a structurally compatible object schema', () => {
  const schema: IFrontmatterSchema = {
    shape: { title: {} },
    toJSONSchema: () => ({
      type: 'object',
      properties: { title: { type: 'string' } },
      required: ['title'],
    }),
  }

  const normalized = normalizeConfig({
    root: '/project',
    collections: { posts: { directory: 'content/posts', schema } },
  })

  assert.deepEqual(normalized.collections.posts?.schema, schema.toJSONSchema())
})

test('rejects Zod transforms that JSON Schema cannot represent', () => {
  assert.throws(
    () =>
      normalizeConfig({
        root: '/project',
        collections: {
          posts: {
            directory: 'content/posts',
            schema: z.object({ title: z.string().transform((value) => value.length) }),
          },
        },
      }),
    /AMAMO_CONFIG_INVALID: collections\.posts\.schema Transforms cannot be represented/,
  )
})

test('normalizes math configuration', () => {
  const config = defineConfig({
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: emptySchema,
      },
    },
  })

  assert.deepEqual(normalizeConfig(config).mdx.math, {
    enabled: false,
    macros: {},
    singleDollar: true,
  })
  assert.deepEqual(normalizeConfig({ ...config, mdx: { math: false } }).mdx.math, {
    enabled: false,
    macros: {},
    singleDollar: true,
  })
  assert.deepEqual(normalizeConfig({ ...config, mdx: { math: {} } }).mdx.math, {
    enabled: true,
    macros: {},
    singleDollar: true,
  })

  const macros = { '\\RR': '\\mathbb{R}' }
  const normalized = normalizeConfig({
    ...config,
    mdx: { math: { macros, singleDollar: false } },
  })
  assert.deepEqual(normalized.mdx.math, {
    enabled: true,
    macros,
    singleDollar: false,
  })
  assert.notEqual(normalized.mdx.math.macros, macros)
})

test('rejects the removed top-level math location', () => {
  const config = {
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: emptySchema,
      },
    },
    math: {},
  }

  assert.throws(
    () => normalizeConfig(config as never),
    /AMAMO_CONFIG_INVALID: math moved to mdx\.math/,
  )
})

test('normalizes optional MDX extensions with GFM inheritance', () => {
  const config = defineConfig({
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: emptySchema,
      },
    },
  })

  assert.deepEqual(normalizeConfig({ ...config, mdx: { gfm: false } }).mdx.extensions, {
    footnotes: false,
    headingIds: false,
    taskLists: false,
  })
  assert.deepEqual(
    normalizeConfig({
      ...config,
      mdx: {
        gfm: false,
        extensions: {
          footnotes: true,
          headingIds: true,
          taskLists: true,
        },
      },
    }).mdx.extensions,
    {
      footnotes: true,
      headingIds: true,
      taskLists: true,
    },
  )
  assert.deepEqual(
    normalizeConfig({
      ...config,
      mdx: { extensions: { footnotes: false, taskLists: false } },
    }).mdx.extensions,
    {
      footnotes: false,
      headingIds: false,
      taskLists: false,
    },
  )
})

test('rejects math macro names without a leading backslash', () => {
  const config = defineConfig({
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: emptySchema,
      },
    },
    mdx: { math: { macros: { RR: '\\mathbb{R}' } } },
  })

  assert.throws(
    () => normalizeConfig(config),
    /AMAMO_CONFIG_INVALID: mdx\.math\.macros\.RR must start with a backslash/,
  )
})

test('bounds configured math macro bytes', () => {
  const config = defineConfig({
    root: '/project',
    collections: {
      posts: {
        directory: 'content/posts',
        schema: emptySchema,
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
    normalizeConfig({ ...config, mdx: { math: { macros: exactLimit } } }).mdx.math.macros,
    exactLimit,
  )
  assert.throws(
    () =>
      normalizeConfig({
        ...config,
        mdx: { math: { macros: { '\\oversized': 'é'.repeat(513) } } },
      }),
    /AMAMO_CONFIG_INVALID: mdx\.math\.macros\.\\oversized exceeds the 1024-byte limit/,
  )
  assert.throws(
    () =>
      normalizeConfig({
        ...config,
        mdx: { math: { macros: { ...exactLimit, '\\extra': '' } } },
      }),
    /AMAMO_CONFIG_INVALID: mdx\.math\.macros exceeds the 16384-byte total limit/,
  )
})
