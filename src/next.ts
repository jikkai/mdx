import path from 'node:path'
import { fileURLToPath } from 'node:url'

import type { NextConfig } from 'next'
import { PHASE_DEVELOPMENT_SERVER, PHASE_PRODUCTION_BUILD } from 'next/constants.js'

import type { IAmamoMdxConfig } from './config.js'
import { normalizeConfig } from './config.js'
import type { IAdapterCompiler } from './compiler.js'
import { createAdapterCompiler } from './compiler.js'
import { configurationFingerprint } from './native.js'

export interface IAmamoNextConfigContext {
  defaultConfig: NextConfig
}

export type AmamoNextConfigInput =
  | NextConfig
  | Promise<NextConfig>
  | ((phase: string, context: IAmamoNextConfigContext) => NextConfig | Promise<NextConfig>)

export type AmamoNextConfig = (
  phase: string,
  context: IAmamoNextConfigContext,
) => Promise<NextConfig>

export function withAmamoMdx(config: IAmamoMdxConfig) {
  const normalized = normalizeConfig(config)
  const loader = fileURLToPath(new URL('./next-loader.cjs', import.meta.url))
  const loaderOptions = {
    configFingerprint: configurationFingerprint(normalized),
    indexFile: path.join(normalized.generatedDirectory, 'index.json'),
  }
  let compilerPromise: Promise<IAdapterCompiler> | undefined
  let buildPromise: Promise<unknown> | undefined
  let watching = false

  function compiler(): Promise<IAdapterCompiler> {
    compilerPromise ??= createAdapterCompiler(config)
    return compilerPromise
  }

  async function prepare(phase: string): Promise<void> {
    if (phase !== PHASE_DEVELOPMENT_SERVER && phase !== PHASE_PRODUCTION_BUILD) return
    buildPromise ??= compiler().then((instance) => instance.build())
    await buildPromise
    if (phase === PHASE_DEVELOPMENT_SERVER && !watching) {
      const instance = await compiler()
      instance.startWatch()
      watching = true
    }
  }

  return function applyAmamoMdx(nextConfig: AmamoNextConfigInput = {}): AmamoNextConfig {
    return async (phase, context) => {
      const resolved =
        typeof nextConfig === 'function' ? await nextConfig(phase, context) : await nextConfig
      await prepare(phase)
      const existingWebpack = resolved.webpack

      return {
        ...resolved,
        turbopack: {
          ...resolved.turbopack,
          rules: {
            ...resolved.turbopack?.rules,
            '*.mdx': {
              as: '*.js',
              loaders: [{ loader, options: loaderOptions }],
            },
          },
        },
        webpack(webpackConfig, webpackContext) {
          const configured = existingWebpack
            ? existingWebpack(webpackConfig, webpackContext)
            : webpackConfig
          configured.module ??= {}
          configured.module.rules ??= []
          configured.module.rules.push({
            test: /\.mdx$/,
            use: [{ loader, options: loaderOptions }],
          })
          return configured
        },
      }
    }
  }
}
