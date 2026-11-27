import assert from 'node:assert/strict'
import { access, readFile, realpath, unlink, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import path from 'node:path'
import type { NextConfig } from 'next'
import { PHASE_PRODUCTION_BUILD } from 'next/constants.js'
import { test } from 'vitest'
import { createAdapterCompiler } from '../compiler.js'
import { withAmamoMdx } from '../next.js'
import { createCompilerFixture } from './fixture.js'

interface ILoaderOptions {
  configFingerprint: string
  indexFile: string
}

interface ILoaderContext {
  addDependency(file: string): void
  getOptions(): ILoaderOptions
  resourcePath: string
}

interface ILoaderIndex {
  cacheDirectory: string
  configFingerprint: string
  documents: Record<string, { cacheKey: string }>
}

type NextLoader = (this: ILoaderContext, source: string) => Promise<string>

const nextLoader = createRequire(import.meta.url)('../../dist/next-loader.cjs') as NextLoader

test('adds the same loader to Turbopack and Webpack while preserving config', async () => {
  const fixture = await createCompilerFixture()
  const wrapped = withAmamoMdx(fixture.config)(async () => ({
    reactStrictMode: true,
    turbopack: {
      rules: {
        '*.txt': { as: '*.js' },
      },
    },
    webpack(config: { marker?: boolean }) {
      config.marker = true
      return config
    },
  }))

  try {
    const config = await wrapped(PHASE_PRODUCTION_BUILD, {
      defaultConfig: {} as NextConfig,
    })
    assert.equal(config.reactStrictMode, true)
    assert.equal(
      config.turbopack?.rules?.['*.mdx'] && 'as' in config.turbopack.rules['*.mdx']
        ? config.turbopack.rules['*.mdx'].as
        : undefined,
      '*.js',
    )
    assert.ok(config.turbopack?.rules?.['*.txt'])

    const webpackConfig = config.webpack?.({ module: { rules: [] } }, {} as never)
    assert.equal(webpackConfig.marker, true)
    assert.match(JSON.stringify(webpackConfig), /next-loader\.cjs/)

    const rule = config.turbopack?.rules?.['*.mdx']
    assert.ok(rule && !Array.isArray(rule) && rule.loaders)
    const loaderItem = rule.loaders[0]
    assert.ok(loaderItem && typeof loaderItem === 'object' && loaderItem.options)
    const options = loaderItem.options as unknown as ILoaderOptions
    assert.match(await runLoader(fixture.post, options, fixture.source), /Hello/)
    await assert.rejects(
      runLoader(fixture.post, { ...options, configFingerprint: 'stale' }, fixture.source),
      /AMAMO_NEXT_CACHE_MISS/,
    )
  } finally {
    await fixture.cleanup()
  }
})

test('compiles a Next cache miss on demand and watches unresolved media', async () => {
  const fixture = await createCompilerFixture()
  fixture.config.media = { missing: 'warn' }
  const image = await realpath(path.join(path.dirname(fixture.post), 'image.png'))
  const compiler = await createAdapterCompiler(fixture.config, { retainModules: false })
  try {
    await unlink(image)
    const result = await compiler.scan()
    assert.equal(result.compiled, 0)
    const indexFile = path.join(fixture.generatedDirectory, 'index.json')
    const index = JSON.parse(await readFile(indexFile, 'utf8')) as ILoaderIndex
    const document = index.documents[await realpath(fixture.post)]
    assert.ok(document)
    const cacheFile = path.join(
      index.cacheDirectory,
      document.cacheKey.slice(0, 2),
      `${document.cacheKey}.json`,
    )
    await assert.rejects(access(cacheFile), { code: 'ENOENT' })

    const options = { configFingerprint: index.configFingerprint, indexFile }
    const watched = new Set<string>()
    const code = await runLoader(fixture.post, options, fixture.source, (file) => watched.add(file))
    assert.ok(watched.has(image))
    assert.match(code, /Hello/)
    await access(cacheFile)
    await writeFile(image, 'restored')
    assert.match(await runLoader(fixture.post, options, fixture.source), /import .*image\.png/)
  } finally {
    await compiler.dispose()
    await fixture.cleanup()
  }
})

function runLoader(
  resourcePath: string,
  options: ILoaderOptions,
  source: string,
  addDependency: (file: string) => void = () => {},
): Promise<string> {
  return nextLoader.call(
    {
      addDependency,
      getOptions: () => options,
      resourcePath,
    },
    source,
  )
}
