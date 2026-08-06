import assert from 'node:assert/strict'
import { createRequire } from 'node:module'

import type { NextConfig } from 'next'
import { PHASE_PRODUCTION_BUILD } from 'next/constants.js'
import { test } from 'vitest'

import { withAmamoMdx } from '../next.js'
import { createCompilerFixture } from './fixture.js'

interface ILoaderOptions {
  configFingerprint: string
  indexFile: string
}

interface ILoaderContext {
  getOptions(): ILoaderOptions
  resourcePath: string
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
    assert.match(await runLoader(fixture.post, options), /Hello/)
    await assert.rejects(
      runLoader(fixture.post, { ...options, configFingerprint: 'stale' }),
      /AMAMO_NEXT_CACHE_MISS/,
    )
  } finally {
    await fixture.cleanup()
  }
})

function runLoader(resourcePath: string, options: ILoaderOptions): Promise<string> {
  return nextLoader.call(
    {
      getOptions: () => options,
      resourcePath,
    },
    '',
  )
}
