import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { access, readFile, unlink, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'
import { test, vi } from 'vitest'
import { createAdapterCompiler, createCompiler } from '../compiler.js'
import { createCompilerFixture } from './fixture.js'

const execFileAsync = promisify(execFile)

test('build, warm build, update, and delete share one document index', async () => {
  const fixture = await createCompilerFixture()
  const compiler = await createCompiler(fixture.config)

  try {
    const first = await compiler.build()
    assert.equal(first.discovered, 1)
    assert.equal(first.compiled, 1)
    assert.equal(first.cached, 0)
    assert.match(
      await readFile(fixture.collectionModule, 'utf8'),
      /import\("\.\.\/content\/posts\/hello\.mdx"\)/,
    )
    assert.match(await readFile(fixture.collectionModule, 'utf8'), /article/)
    assert.doesNotMatch(await readFile(fixture.publicManifest, 'utf8'), /article/)
    const serverManifest = JSON.parse(await readFile(fixture.serverManifest, 'utf8'))
    assert.equal(serverManifest.hello.category, 'article')
    assert.equal(serverManifest.hello.categorized, true)

    const unchanged = await compiler.build()
    assert.equal(unchanged.compiled, 0)
    assert.equal(unchanged.cached, 1)
    assert.equal(unchanged.outputsWritten, 0)

    await writeFile(fixture.post, fixture.updatedSource)
    const transformed = await compiler.transform(fixture.post)
    assert.equal(transformed.cached, false)
    assert.match(transformed.code, /Updated/)
    assert.match(await readFile(fixture.publicManifest, 'utf8'), /Updated/)

    await unlink(fixture.post)
    await compiler.remove(fixture.post)
    assert.doesNotMatch(await readFile(fixture.collectionModule, 'utf8'), /hello\.mdx/)

    const generated = [fixture.collectionModule, fixture.publicManifest, fixture.serverManifest]
    const generatedSources = await Promise.all(generated.map((file) => readFile(file, 'utf8')))
    generatedSources.forEach((source) => assert.doesNotMatch(source, /article/))
  } finally {
    await compiler.dispose()
    await fixture.cleanup()
  }
})

test('incremental edits and removals preserve cached documents outside the current compiler', async () => {
  const fixture = await createCompilerFixture()
  const compiler = await createCompiler(fixture.config)
  const partial = await createCompiler(fixture.config)
  const other = path.join(path.dirname(fixture.post), 'another.mdx')
  try {
    await writeFile(other, fixture.source)
    await compiler.build()
    const previous = await partial.transform(fixture.post)
    const entry = (key: string) =>
      path.join(fixture.generatedDirectory, 'cache', key.slice(0, 2), `${key}.cache`)
    await writeFile(fixture.post, fixture.updatedSource)
    const updated = await partial.transform(fixture.post)
    await assert.rejects(access(entry(previous.record.cacheKey)), { code: 'ENOENT' })
    assert.equal((await compiler.transform(other)).cached, true)

    await partial.remove(fixture.post)
    await assert.rejects(access(entry(updated.record.cacheKey)), { code: 'ENOENT' })
    assert.equal((await compiler.transform(other)).cached, true)
  } finally {
    await partial.dispose()
    await compiler.dispose()
    await fixture.cleanup()
  }
})

test('uses the latest Git commit time for derived lastModified', async () => {
  const fixture = await createCompilerFixture()
  fixture.config.derived = { lastModified: true }
  const manifest = fixture.config.manifests?.public
  assert.ok(manifest)
  manifest.fields.lastModified = 'derived.lastModified'
  const committedAt = '2020-01-02T03:04:05Z'

  try {
    await execFileAsync('git', ['init', '-q'], { cwd: fixture.root })
    await execFileAsync('git', ['add', '.'], { cwd: fixture.root })
    await execFileAsync(
      'git',
      ['-c', 'user.name=Test', '-c', 'user.email=test@example.com', 'commit', '-qm', 'fixture'],
      {
        cwd: fixture.root,
        env: {
          ...process.env,
          GIT_AUTHOR_DATE: committedAt,
          GIT_COMMITTER_DATE: committedAt,
        },
      },
    )
    const compiler = await createCompiler(fixture.config)
    try {
      await compiler.build()
      const entries = JSON.parse(await readFile(fixture.publicManifest, 'utf8'))
      assert.equal(entries[0].lastModified, committedAt)
    } finally {
      await compiler.dispose()
    }
  } finally {
    await fixture.cleanup()
  }
})

test('a corrected document updates the registry after a watcher error', async () => {
  const fixture = await createCompilerFixture()
  const compiler = await createAdapterCompiler(fixture.config, { retainModules: false })
  try {
    await compiler.scan()
    let watchError: Error | undefined
    compiler.startWatch(undefined, (error) => {
      watchError = error
    })
    await writeFile(fixture.post, '---\ntitle: 42\n---\n')
    await vi.waitFor(() => assert.match(watchError?.message ?? '', /title/))
    await writeFile(fixture.post, fixture.updatedSource)
    await vi.waitFor(async () => {
      assert.match(await readFile(fixture.collectionModule, 'utf8'), /Updated/)
    })
  } finally {
    await compiler.dispose()
    await fixture.cleanup()
  }
})

test('strict TypeScript consumers can import the generated collection module', async () => {
  const fixture = await createCompilerFixture()
  const compiler = await createCompiler(fixture.config)
  try {
    await compiler.build()
    const consumer = path.join(fixture.root, 'consumer.mts')
    await writeFile(
      consumer,
      "import { collections } from './.amamo-mdx/collections.mjs'\n" +
        'export const keys: string[] = collections.posts.map((doc) => doc.key)\n',
    )
    const tsc = fileURLToPath(new URL('bin/tsc', import.meta.resolve('typescript/package.json')))
    await assert.doesNotReject(
      Promise.all(
        ['nodenext', 'bundler'].map((resolution) =>
          execFileAsync(process.execPath, [
            tsc,
            '--ignoreConfig',
            '--strict',
            '--noEmit',
            '--skipLibCheck',
            '--module',
            resolution === 'bundler' ? 'esnext' : 'nodenext',
            '--moduleResolution',
            resolution,
            consumer,
          ]),
        ),
      ),
    )
  } finally {
    await compiler.dispose()
    await fixture.cleanup()
  }
})

test('adding a missing image refreshes cached MDX without editing its source', async () => {
  const fixture = await createCompilerFixture()
  fixture.config.media = { missing: 'warn' }
  const compiler = await createAdapterCompiler(fixture.config)
  const image = path.join(path.dirname(fixture.post), 'image.png')
  try {
    await unlink(image)
    const missing = await compiler.transformSource(fixture.post, fixture.source)
    assert.ok(missing.record.diagnostics.some(({ code }) => code === 'AMAMO_MEDIA_MISSING'))
    assert.equal((await compiler.build()).cached, 1)

    await writeFile(image, 'restored')
    const restored = await compiler.transformSource(fixture.post, fixture.source)
    assert.match(restored.code, /import .*image\.png/)
    assert.ok(!restored.record.diagnostics.some(({ code }) => code === 'AMAMO_MEDIA_MISSING'))
    assert.equal((await compiler.build()).cached, 1)

    await unlink(image)
    const removed = await compiler.transformSource(fixture.post, fixture.source)
    assert.ok(removed.record.diagnostics.some(({ code }) => code === 'AMAMO_MEDIA_MISSING'))
  } finally {
    await compiler.dispose()
    await fixture.cleanup()
  }
})
