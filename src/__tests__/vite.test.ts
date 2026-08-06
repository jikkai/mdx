import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { readFile, stat, unlink, writeFile } from 'node:fs/promises'
import path from 'node:path'

import { build, createServer } from 'vite'
import { test } from 'vitest'

import { amamoMdx } from '../vite.js'
import { createCompilerFixture } from './fixture.js'

const reactJsxRuntime = createRequire(import.meta.url).resolve('react/jsx-runtime')

test('Vite builds MDX without rewriting unchanged generated outputs', async () => {
  const fixture = await createCompilerFixture()
  await writeFile(
    path.join(fixture.root, 'index.html'),
    '<div id="root"></div><script type="module" src="/main.js"></script>',
  )
  await writeFile(
    path.join(fixture.root, 'main.js'),
    "import Post from './content/posts/hello.mdx'; console.log(Post)\n",
  )

  try {
    await build({
      root: fixture.root,
      logLevel: 'silent',
      plugins: [amamoMdx(fixture.config)],
      resolve: { alias: { 'react/jsx-runtime': reactJsxRuntime } },
    })
    assert.match(await readFile(path.join(fixture.root, 'dist/index.html'), 'utf8'), /root/)
    assert.equal(JSON.parse(await readFile(fixture.publicManifest, 'utf8')).length, 1)
    const firstWrite = (await stat(fixture.publicManifest)).mtimeMs

    await build({
      root: fixture.root,
      logLevel: 'silent',
      plugins: [amamoMdx(fixture.config)],
      resolve: { alias: { 'react/jsx-runtime': reactJsxRuntime } },
    })
    assert.equal((await stat(fixture.publicManifest)).mtimeMs, firstWrite)
  } finally {
    await fixture.cleanup()
  }
})

test('Vite dev watcher updates generated output for create, update, and delete', async () => {
  const fixture = await createCompilerFixture()
  const secondPost = path.join(path.dirname(fixture.post), 'second.mdx')
  const server = await createServer({
    root: fixture.root,
    logLevel: 'silent',
    plugins: [amamoMdx(fixture.config)],
    resolve: { alias: { 'react/jsx-runtime': reactJsxRuntime } },
    server: { host: '127.0.0.1', port: 0 },
  })

  try {
    await server.listen()
    await writeFile(fixture.post, fixture.updatedSource)
    await waitFor(async () => (await readFile(fixture.publicManifest, 'utf8')).includes('Updated'))

    await writeFile(secondPost, '---\ntitle: Second\n---\n# Second\n')
    await waitFor(
      async () => JSON.parse(await readFile(fixture.publicManifest, 'utf8')).length === 2,
    )

    await unlink(secondPost)
    await waitFor(async () => !(await readFile(fixture.publicManifest, 'utf8')).includes('Second'))
    assert.doesNotMatch(await readFile(fixture.publicManifest, 'utf8'), /Second/)
  } finally {
    await server.close()
    await fixture.cleanup()
  }
})

async function waitFor(check: () => Promise<boolean>): Promise<void> {
  const deadline = Date.now() + 5_000
  while (Date.now() < deadline) {
    if (await check()) return
    await new Promise((resolve) => setTimeout(resolve, 20))
  }
  throw new Error('Timed out waiting for Vite watcher')
}
