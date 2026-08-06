import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'

import { test } from 'vitest'

import { normalizeConfig } from '../config.js'
import {
  AmamoMdxError,
  prepareNativeBatch,
  pruneNativeCache,
  renderNativeManifests,
} from '../native.js'
import { createShikiRenderer } from '../shiki.js'

const config = normalizeConfig({
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
})

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

test('injects real Shiki HAST into the compiled module', async () => {
  const batch = prepareNativeBatch(config, [
    {
      collection: 'posts',
      file: '/project/content/posts/code.mdx',
      key: 'code',
      source: '---\ntitle: Code\n---\n```ts\nconst n: number = 1\n```\n',
    },
  ])
  assert.equal(batch.codeBlocks.length, 1)
  const renderer = await createShikiRenderer(config.highlight)

  try {
    const records = batch.finish(await renderer.highlight(batch.codeBlocks))
    assert.match(records[0]?.module ?? '', /--shiki-dark/)
    assert.match(records[0]?.module ?? '', /style: \{/)
    assert.doesNotMatch(records[0]?.module ?? '', /style: "--shiki-dark/)
    assert.doesNotMatch(records[0]?.module ?? '', /language-ts/)
  } finally {
    renderer.dispose()
  }
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
