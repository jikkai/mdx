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
