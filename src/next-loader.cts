import { readFile, realpath } from 'node:fs/promises'
import path from 'node:path'
import type { IAmamoMDXConfig } from './config.js'
import type { INativeDocumentInput } from './native.js'

interface INextLoaderOptions {
  configFingerprint: string
  indexFile: string
}

interface INextLoaderContext {
  addDependency(file: string): void
  getOptions(): INextLoaderOptions
  resourcePath: string
}

interface INextLoaderDocument {
  cacheKey: string
  collection: string
  key: string
  locale?: string
  modifiedAt?: string
  slug?: string
}

interface INextLoaderIndex {
  cacheDirectory: string
  config: IAmamoMDXConfig
  configFingerprint: string
  documents: Record<string, INextLoaderDocument>
  version: number
}

function nextLoader(this: INextLoaderContext, source: string): Promise<string> {
  return compileModule(this, source)
}

async function compileModule(context: INextLoaderContext, source: string): Promise<string> {
  const options = context.getOptions()
  context.addDependency(options.indexFile)
  let config: IAmamoMDXConfig
  let input: INativeDocumentInput
  try {
    const index = JSON.parse(await readFile(options.indexFile, 'utf8')) as INextLoaderIndex
    const resource = await realpath(path.resolve(context.resourcePath))
    const document = index.documents?.[resource]
    if (
      index.version !== 2 ||
      index.configFingerprint !== options.configFingerprint ||
      typeof index.cacheDirectory !== 'string' ||
      typeof index.config !== 'object' ||
      index.config === null ||
      index.config.cache.directory !== index.cacheDirectory ||
      typeof document?.cacheKey !== 'string' ||
      !/^[a-f0-9]{64}$/.test(document.cacheKey) ||
      typeof document.collection !== 'string' ||
      typeof document.key !== 'string'
    ) {
      throw new Error('index entry does not match this build')
    }
    config = index.config
    input = {
      collection: document.collection,
      file: resource,
      key: document.key,
      locale: document.locale,
      modifiedAt: document.modifiedAt,
      slug: document.slug,
      source,
    }
  } catch (error) {
    throw new Error(
      `AMAMO_NEXT_CACHE_MISS: ${context.resourcePath}; rerun the coordinated amamo-mdx build`,
      { cause: error },
    )
  }
  const { compileNextDocument } = await import('./next-runtime.js')
  const record = await compileNextDocument(config, input)
  for (const dependency of [...record.dependencies, ...record.missingDependencies]) {
    context.addDependency(dependency)
  }
  return record.module
}

export = nextLoader
