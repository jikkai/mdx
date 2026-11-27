import { createHash } from 'node:crypto'
import { createRequire } from 'node:module'
import type { IAmamoMDXConfig, JsonValue } from './config.js'

export interface ISourcePoint {
  column: number
  line: number
  offset: number
}

export interface ISourceRange {
  end: ISourcePoint
  start: ISourcePoint
}

export interface IDiagnostic {
  code: string
  file?: string
  hint?: string
  message: string
  range?: ISourceRange
  severity: 'error' | 'warning'
}

export interface INativeDocumentInput {
  collection: string
  file: string
  key: string
  locale?: string
  modifiedAt?: string
  slug?: string
  source: string
}

export interface ICodeBlock {
  blockId: number
  code: string
  documentId: string
  lang: string | null
  meta: string | null
}

export interface IHighlightedCodeBlock {
  blockId: number
  documentId: string
  hast: JsonValue
}

export interface IDocumentMetadata {
  cacheKey: string
  cached: boolean
  collection: string
  dependencies: string[]
  missingDependencies: string[]
  derived: Record<string, JsonValue>
  diagnostics: IDiagnostic[]
  file: string
  frontmatter: Record<string, JsonValue>
  hash: string
  key: string
  locale?: string
  modifiedAt?: string
  projections: Record<string, JsonValue>
  slug?: string
}

export interface IDocumentRecord extends IDocumentMetadata {
  module: string
}

export interface IRenderedManifest {
  contents: string
  path: string
}

interface INativePreparedBatch {
  readonly codeBlocksJson: string
  finish(highlightsJson: string): string
  takeMetadataJson(): string
}

interface INativeBinding {
  prepareBatch(configJson: string, inputsJson: string): INativePreparedBatch
  pruneCache(cacheDirectory: string, keepKeysJson: string): number
  removeCacheEntry(cacheDirectory: string, key: string): void
  renderManifests(configJson: string, recordsJson: string): string
}

export interface IPreparedNativeBatch {
  codeBlocks: ICodeBlock[]
  finish(highlights: IHighlightedCodeBlock[]): IDocumentRecord[]
  takeMetadata(): IDocumentMetadata[]
}

export class AmamoMdxError extends Error {
  public readonly diagnostics: IDiagnostic[]

  public constructor(diagnostics: IDiagnostic[]) {
    super(diagnostics.map((diagnostic) => diagnostic.message).join('\n'))
    this.name = 'AmamoMdxError'
    this.diagnostics = diagnostics
  }
}

const require = createRequire(import.meta.url)
const packageVersion = (require('../package.json') as { version: string }).version
const shikiVersion = (require('shiki/package.json') as { version: string }).version

function nativeConfigJson(config: IAmamoMDXConfig): string {
  return JSON.stringify({
    ...config,
    _runtime: { packageVersion, shikiTransformers: true, shikiVersion },
  })
}

export function configurationFingerprint(config: IAmamoMDXConfig): string {
  return createHash('sha256').update(nativeConfigJson(config)).digest('hex')
}

function loadBinding(): INativeBinding {
  return require('../native.cjs') as INativeBinding
}

function mapNativeError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error)
  const marker = 'AMAMO_MDX_DIAGNOSTICS:'
  const index = message.indexOf(marker)
  if (index === -1) throw error

  throw new AmamoMdxError(JSON.parse(message.slice(index + marker.length)) as IDiagnostic[])
}

export function prepareNativeBatch(
  config: IAmamoMDXConfig,
  inputs: INativeDocumentInput[],
): IPreparedNativeBatch {
  try {
    const batch = loadBinding().prepareBatch(nativeConfigJson(config), JSON.stringify(inputs))
    return {
      codeBlocks: JSON.parse(batch.codeBlocksJson) as ICodeBlock[],
      finish(highlights) {
        try {
          return JSON.parse(batch.finish(JSON.stringify(highlights))) as IDocumentRecord[]
        } catch (error) {
          return mapNativeError(error)
        }
      },
      takeMetadata() {
        try {
          return JSON.parse(batch.takeMetadataJson()) as IDocumentMetadata[]
        } catch (error) {
          return mapNativeError(error)
        }
      },
    }
  } catch (error) {
    return mapNativeError(error)
  }
}

export function removeNativeCacheEntry(cacheDirectory: string, key: string): void {
  try {
    loadBinding().removeCacheEntry(cacheDirectory, key)
  } catch (error) {
    mapNativeError(error)
  }
}

export function pruneNativeCache(cacheDirectory: string, keepKeys: string[]): number {
  try {
    return loadBinding().pruneCache(cacheDirectory, JSON.stringify(keepKeys))
  } catch (error) {
    return mapNativeError(error)
  }
}

export function renderNativeManifests(
  config: IAmamoMDXConfig,
  records: IDocumentMetadata[],
): IRenderedManifest[] {
  try {
    const manifestRecords = records.map(({ key, projections }) => ({
      key,
      projections,
    }))
    return JSON.parse(
      loadBinding().renderManifests(nativeConfigJson(config), JSON.stringify(manifestRecords)),
    ) as IRenderedManifest[]
  } catch (error) {
    return mapNativeError(error)
  }
}
