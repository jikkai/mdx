import { readFile, realpath } from 'node:fs/promises'
import path from 'node:path'

interface INextLoaderOptions {
  configFingerprint: string
  indexFile: string
}

interface INextLoaderContext {
  getOptions(): INextLoaderOptions
  resourcePath: string
}

interface INextLoaderIndex {
  cacheDirectory: string
  configFingerprint: string
  documents: Record<string, { cacheKey: string }>
}

function nextLoader(this: INextLoaderContext, _source: string): Promise<string> {
  return readCompiledModule(this)
}

async function readCompiledModule(context: INextLoaderContext): Promise<string> {
  const options = context.getOptions()
  try {
    const index = JSON.parse(await readFile(options.indexFile, 'utf8')) as INextLoaderIndex
    const resource = await realpath(path.resolve(context.resourcePath))
    const cacheKey = index.documents?.[resource]?.cacheKey
    if (
      index.configFingerprint !== options.configFingerprint ||
      typeof index.cacheDirectory !== 'string' ||
      typeof cacheKey !== 'string' ||
      !/^[a-f0-9]{64}$/.test(cacheKey)
    ) {
      throw new Error('index entry does not match this build')
    }
    const cacheFile = path.join(index.cacheDirectory, cacheKey.slice(0, 2), `${cacheKey}.json`)
    const record = JSON.parse(await readFile(cacheFile, 'utf8')) as {
      cacheKey?: unknown
      module?: unknown
    }
    if (record.cacheKey !== cacheKey || typeof record.module !== 'string') {
      throw new Error('cache record does not match its index entry')
    }
    return record.module
  } catch (error) {
    throw new Error(
      `AMAMO_NEXT_CACHE_MISS: ${context.resourcePath}; rerun the coordinated amamo-mdx build`,
      { cause: error },
    )
  }
}

export = nextLoader
