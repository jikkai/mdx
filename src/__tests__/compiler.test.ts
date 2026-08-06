import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { readFile, unlink, writeFile } from 'node:fs/promises'
import { promisify } from 'node:util'

import { test } from 'vitest'

import { createCompiler } from '../compiler.js'
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
    assert.doesNotMatch(await readFile(fixture.publicManifest, 'utf8'), /secret/)
    assert.doesNotMatch(await readFile(fixture.serverManifest, 'utf8'), /secret/)

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
    generatedSources.forEach((source) => assert.doesNotMatch(source, /secret/))
  } finally {
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
