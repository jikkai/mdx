import type { FSWatcher } from 'node:fs'
import type { FileHandle } from 'node:fs/promises'
import { execFile } from 'node:child_process'
import { realpathSync, watch } from 'node:fs'
import { mkdir, open, readFile, readdir, rename, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

import type { IAmamoMDXConfig, IAmamoMdxConfig, INormalizedCollectionConfig } from './config.js'
import { normalizeConfig } from './config.js'
import type {
  IDocumentMetadata,
  IDocumentRecord,
  IHighlightedCodeBlock,
  INativeDocumentInput,
} from './native.js'
import {
  configurationFingerprint,
  prepareNativeBatch,
  pruneNativeCache,
  renderNativeManifests,
} from './native.js'
import type { IShikiRenderer } from './shiki.js'
import { createShikiRenderer } from './shiki.js'

export interface IBuildResult {
  cached: number
  compiled: number
  discovered: number
  outputsWritten: number
}

export interface ITransformResult {
  cached: boolean
  code: string
  map: null
  outputsWritten: number
  record: IDocumentRecord
}

export interface ICompiler {
  build(): Promise<IBuildResult>
  dispose(): Promise<void>
  remove(file: string): Promise<number>
  transform(file: string): Promise<ITransformResult>
}

export interface IAdapterCompiler extends ICompiler {
  readonly generatedCollectionModule: string
  isContentFile(file: string): boolean
  scan(): Promise<IBuildResult>
  startWatch(onGeneratedChange?: (paths: string[]) => Promise<void> | void): () => void
  transformSource(file: string, source: string): Promise<ITransformResult>
}

interface IFileMetadata {
  collection: string
  file: string
  key: string
  locale?: string
  slug: string
}

interface IDiscoveredDocument {
  input: INativeDocumentInput
}

let temporaryFileId = 0
const execFileAsync = promisify(execFile)

class Compiler implements IAdapterCompiler {
  public readonly generatedCollectionModule: string

  private buildInFlight: Promise<IBuildResult> | undefined
  private disposed = false
  private generatedListener: ((paths: string[]) => Promise<void> | void) | undefined
  private readonly gitModifiedTimes = new Map<string, string>()
  private readonly modules = new Map<string, string>()
  private readonly records = new Map<string, IDocumentMetadata>()
  private readonly sources = new Map<string, string>()
  private shikiPromise: Promise<IShikiRenderer> | undefined
  private tail: Promise<void> = Promise.resolve()
  private watcher: FSWatcher | undefined
  private watcherError: Error | undefined

  public constructor(
    private readonly config: IAmamoMDXConfig,
    private readonly retainModules: boolean,
  ) {
    this.generatedCollectionModule = path.join(config.generatedDirectory, 'collections.mjs')
  }

  public build(): Promise<IBuildResult> {
    if (this.buildInFlight) return this.buildInFlight
    const operation = this.enqueue(() => this.buildNow())
    const tracked = operation.then(
      (result) => {
        this.buildInFlight = undefined
        return result
      },
      (error: unknown) => {
        this.buildInFlight = undefined
        throw error
      },
    )
    this.buildInFlight = tracked
    return tracked
  }

  public transform(file: string): Promise<ITransformResult> {
    return this.enqueue(() => this.transformNow(cleanFileId(file)))
  }

  public transformSource(file: string, source: string): Promise<ITransformResult> {
    return this.enqueue(() => this.transformNow(cleanFileId(file), source))
  }

  public scan(): Promise<IBuildResult> {
    return this.enqueue(() => this.scanNow())
  }

  public remove(file: string): Promise<number> {
    return this.enqueue(async () => {
      const absolute = cleanFileId(file)
      if (!this.records.delete(absolute)) return 0
      this.modules.delete(absolute)
      this.sources.delete(absolute)
      const changed = await this.writeOutputs()
      if (this.config.cache.enabled) {
        pruneNativeCache(
          this.config.cache.directory,
          [...this.records.values()].map((record) => record.cacheKey),
        )
      }
      await this.notifyGeneratedChange(changed)
      return changed.length
    })
  }

  public async dispose(): Promise<void> {
    if (this.disposed) return
    await this.enqueue(async () => {
      this.watcher?.close()
      this.watcher = undefined
      const shiki = this.shikiPromise ? await this.shikiPromise : undefined
      shiki?.dispose()
      this.disposed = true
    }, false)
  }

  public isContentFile(file: string): boolean {
    return this.metadataForFile(cleanFileId(file)) !== undefined
  }

  public startWatch(onGeneratedChange?: (paths: string[]) => Promise<void> | void): () => void {
    if (this.watcher) return () => this.stopWatch()
    this.generatedListener = onGeneratedChange
    this.watcher = watch(this.config.root, { recursive: true }, async (_event, filename) => {
      if (!filename) return
      const file = path.resolve(this.config.root, filename.toString())
      if (!this.isContentFile(file) && !this.records.has(file)) return
      try {
        await stat(file)
        await this.transform(file)
      } catch (error) {
        if (isMissingFile(error)) {
          await this.remove(file)
        } else {
          this.watcherError = asError(error)
        }
      }
    })
    this.watcher.on('error', (error) => {
      this.watcherError = error
    })
    return () => this.stopWatch()
  }

  private stopWatch(): void {
    this.watcher?.close()
    this.watcher = undefined
    this.generatedListener = undefined
  }

  private enqueue<T>(operation: () => Promise<T>, requireOpen = true): Promise<T> {
    const run = async () => {
      if (requireOpen) this.assertOpen()
      return operation()
    }
    const result = this.tail.then(run, run)
    this.tail = result.then(
      () => undefined,
      () => undefined,
    )
    return result
  }

  private assertOpen(): void {
    if (this.disposed) throw new Error('AMAMO_COMPILER_DISPOSED: Compiler is disposed')
    if (this.watcherError) {
      const error = this.watcherError
      this.watcherError = undefined
      throw error
    }
  }

  private async buildNow(): Promise<IBuildResult> {
    const discovered = await this.discover()
    const records = await this.compile(discovered.map((document) => document.input))
    this.replaceRecords(
      records.map(documentMetadata),
      discovered.map((document) => document.input),
    )
    this.modules.clear()
    if (this.retainModules) {
      for (const record of records) this.modules.set(path.resolve(record.file), record.module)
    }
    const changed = await this.writeOutputs()
    if (this.config.cache.enabled) {
      pruneNativeCache(
        this.config.cache.directory,
        records.map((record) => record.cacheKey),
      )
    }
    await this.notifyGeneratedChange(changed)
    return {
      cached: records.filter((record) => record.cached).length,
      compiled: records.filter((record) => !record.cached).length,
      discovered: discovered.length,
      outputsWritten: changed.length,
    }
  }

  private async scanNow(): Promise<IBuildResult> {
    const discovered = await this.discover()
    const inputs = discovered.map((document) => document.input)
    const records = prepareNativeBatch(this.config, inputs).takeMetadata()
    this.replaceRecords(records, inputs)
    this.modules.clear()
    const changed = await this.writeOutputs()
    if (this.config.cache.enabled) {
      pruneNativeCache(
        this.config.cache.directory,
        records.map((record) => record.cacheKey),
      )
    }
    await this.notifyGeneratedChange(changed)
    return {
      cached: records.filter((record) => record.cached).length,
      compiled: 0,
      discovered: discovered.length,
      outputsWritten: changed.length,
    }
  }

  private async transformNow(file: string, source?: string): Promise<ITransformResult> {
    const current = this.records.get(file)
    const currentModule = this.modules.get(file)
    if (
      source !== undefined &&
      source === this.sources.get(file) &&
      current &&
      currentModule !== undefined
    ) {
      return {
        cached: true,
        code: currentModule,
        map: null,
        outputsWritten: 0,
        record: { ...current, cached: true, module: currentModule },
      }
    }
    const metadata = this.metadataForFile(file)
    if (!metadata) {
      throw new Error(`AMAMO_FILE_OUTSIDE_COLLECTION: ${file}`)
    }
    for (const record of this.records.values()) {
      if (
        record.file !== file &&
        record.collection === metadata.collection &&
        record.key === metadata.key
      ) {
        throw new Error(`AMAMO_DOCUMENT_DUPLICATE_KEY: ${metadata.collection}/${metadata.key}`)
      }
    }
    const document = await this.readDocument(metadata, source)
    const records = await this.compile([document.input])
    const record = records[0]
    if (!record) throw new Error(`AMAMO_NATIVE_RESULT_MISSING: ${file}`)
    this.records.set(file, documentMetadata(record))
    if (this.retainModules) {
      this.modules.set(file, record.module)
      this.sources.set(file, document.input.source)
    }
    const changed = await this.writeOutputs()
    if (this.config.cache.enabled) {
      pruneNativeCache(
        this.config.cache.directory,
        [...this.records.values()].map((value) => value.cacheKey),
      )
    }
    await this.notifyGeneratedChange(changed)
    return {
      cached: record.cached,
      code: record.module,
      map: null,
      outputsWritten: changed.length,
      record,
    }
  }

  private async compile(inputs: INativeDocumentInput[]): Promise<IDocumentRecord[]> {
    const batch = prepareNativeBatch(this.config, inputs)
    let highlights: IHighlightedCodeBlock[] = []
    if (this.config.highlight.enabled && batch.codeBlocks.length > 0) {
      this.shikiPromise ??= createShikiRenderer(this.config.highlight)
      highlights = await (await this.shikiPromise).highlight(batch.codeBlocks)
    }
    return batch.finish(highlights)
  }

  private async discover(): Promise<IDiscoveredDocument[]> {
    const documentsMetadata: IFileMetadata[] = []
    const keys = new Map<string, string>()
    for (const [collection, config] of Object.entries(this.config.collections).toSorted(
      ([left], [right]) => left.localeCompare(right),
    )) {
      for (const file of await walkFiles(config.directory)) {
        const metadata = this.metadataForFile(file, collection)
        if (!metadata) continue
        const duplicateKey = `${metadata.collection}\0${metadata.key}`
        const duplicate = keys.get(duplicateKey)
        if (duplicate) {
          throw new Error(
            `AMAMO_DOCUMENT_DUPLICATE_KEY: ${metadata.collection}/${metadata.key} is shared by ${duplicate} and ${file}`,
          )
        }
        keys.set(duplicateKey, file)
        documentsMetadata.push(metadata)
      }
    }
    this.gitModifiedTimes.clear()
    if (this.config.derived.lastModified) {
      const times = await gitLastModified(
        this.config.root,
        documentsMetadata.map(({ file }) => file),
        Object.values(this.config.collections).map(({ directory }) => directory),
      )
      for (const [file, modifiedAt] of times) this.gitModifiedTimes.set(file, modifiedAt)
    }
    const documents = await Promise.all(documentsMetadata.map((value) => this.readDocument(value)))
    return documents.toSorted((left, right) => left.input.file.localeCompare(right.input.file))
  }

  private async readDocument(
    metadata: IFileMetadata,
    providedSource?: string,
  ): Promise<IDiscoveredDocument> {
    const [source, stats] = await Promise.all([
      providedSource ?? readFile(metadata.file, 'utf8'),
      this.config.derived.lastModified ? stat(metadata.file) : undefined,
    ])
    return {
      input: {
        collection: metadata.collection,
        file: metadata.file,
        key: metadata.key,
        locale: metadata.locale,
        modifiedAt: this.config.derived.lastModified
          ? (this.gitModifiedTimes.get(metadata.file) ?? stats?.mtime.toISOString())
          : undefined,
        slug: metadata.slug,
        source,
      },
    }
  }

  private metadataForFile(file: string, expectedCollection?: string): IFileMetadata | undefined {
    const absolute = path.resolve(file)
    for (const [collection, config] of Object.entries(this.config.collections)) {
      if (expectedCollection && collection !== expectedCollection) continue
      const relative = path.relative(config.directory, absolute)
      if (relative.startsWith('..') || path.isAbsolute(relative)) continue
      const extension = config.extensions.find((value) => relative.endsWith(value))
      if (!extension) continue
      return deriveMetadata(collection, config, absolute, relative.slice(0, -extension.length))
    }
    return undefined
  }

  private async writeOutputs(): Promise<string[]> {
    const records = [...this.records.values()].toSorted(compareRecords)
    const outputs = renderNativeManifests(this.config, records)
    outputs.push(
      {
        contents: generateCollectionModule(this.config, records),
        path: this.generatedCollectionModule,
      },
      {
        contents: generateCollectionTypes(),
        path: path.join(this.config.generatedDirectory, 'collections.d.ts'),
      },
      {
        contents: `${JSON.stringify(
          {
            cacheDirectory: this.config.cache.directory,
            config: this.config,
            configFingerprint: configurationFingerprint(this.config),
            documents: Object.fromEntries(
              records.map((record) => [
                record.file,
                {
                  cacheKey: record.cacheKey,
                  collection: record.collection,
                  key: record.key,
                  locale: record.locale,
                  modifiedAt: record.modifiedAt,
                  slug: record.slug,
                },
              ]),
            ),
            version: 2,
          },
          null,
          2,
        )}\n`,
        path: path.join(this.config.generatedDirectory, 'index.json'),
      },
    )
    const changed: string[] = []
    for (const output of outputs.toSorted((left, right) => left.path.localeCompare(right.path))) {
      if (await writeIfChanged(output.path, output.contents)) changed.push(output.path)
    }
    return changed
  }

  private replaceRecords(records: IDocumentMetadata[], inputs: INativeDocumentInput[]): void {
    this.records.clear()
    for (const record of records) this.records.set(path.resolve(record.file), record)
    this.sources.clear()
    if (this.retainModules) {
      for (const input of inputs) this.sources.set(path.resolve(input.file), input.source)
    }
  }

  private async notifyGeneratedChange(paths: string[]): Promise<void> {
    if (paths.length === 0 || !this.generatedListener) return
    await this.generatedListener(paths)
  }
}

export async function createCompiler(config: IAmamoMdxConfig): Promise<ICompiler> {
  return createAdapterCompiler(config)
}

export async function createAdapterCompiler(
  config: IAmamoMdxConfig,
  options: { retainModules?: boolean } = {},
): Promise<IAdapterCompiler> {
  const normalized = normalizeConfig(config)
  return new Compiler(normalized, options.retainModules ?? true)
}

function deriveMetadata(
  collection: string,
  config: INormalizedCollectionConfig,
  file: string,
  extensionless: string,
): IFileMetadata {
  const segments = extensionless.split(path.sep)
  let name = segments.pop() ?? ''
  let locale: string | undefined
  if (config.locales) {
    locale = config.locales.default
    for (const candidate of config.locales.names.toSorted(
      (left, right) => right.length - left.length,
    )) {
      const suffix = `.${candidate}`
      if (name.endsWith(suffix)) {
        name = name.slice(0, -suffix.length)
        locale = candidate
        break
      }
    }
  }
  if (!config.slug.indexNames.includes(name)) segments.push(name)
  const slug = segments.filter(Boolean).join('/') || '/'
  return {
    collection,
    file,
    key: locale ? `${locale}:${slug}` : slug,
    locale,
    slug,
  }
}

async function walkFiles(directory: string): Promise<string[]> {
  const files: string[] = []
  const entries = await readdir(directory, { withFileTypes: true })
  for (const entry of entries.toSorted((left, right) => left.name.localeCompare(right.name))) {
    const file = path.join(directory, entry.name)
    if (entry.isDirectory()) files.push(...(await walkFiles(file)))
    else if (entry.isFile()) files.push(file)
  }
  return files
}

function generateCollectionModule(config: IAmamoMDXConfig, records: IDocumentMetadata[]): string {
  const collections = Object.keys(config.collections).toSorted()
  const lines = ['export const collections = {']
  for (const collection of collections) {
    lines.push(`  ${JSON.stringify(collection)}: [`)
    for (const record of records.filter((value) => value.collection === collection)) {
      const specifier = importSpecifier(config.generatedDirectory, record.file)
      const metadata = JSON.stringify({
        derived: record.derived,
        frontmatter: record.frontmatter,
        key: record.key,
        locale: record.locale,
        slug: record.slug,
      })
      lines.push(`    { ...${metadata}, load: () => import(${JSON.stringify(specifier)}) },`)
    }
    lines.push('  ],')
  }
  lines.push('};', 'export default collections;', '')
  return lines.join('\n')
}

function generateCollectionTypes(): string {
  return `import type { IStructuredData, ITocItem } from '@amamo/mdx'

export interface IGeneratedDocument {
  readonly derived: Readonly<Record<string, unknown>>
  readonly frontmatter: Readonly<Record<string, unknown>>
  readonly key: string
  readonly locale?: string
  readonly slug?: string
  readonly load: () => Promise<{
    readonly default: unknown
    readonly frontmatter: Readonly<Record<string, unknown>>
    readonly structuredData: IStructuredData
    readonly toc: readonly ITocItem[]
  }>
}

export declare const collections: Readonly<Record<string, readonly IGeneratedDocument[]>>
export default collections
`
}

function importSpecifier(from: string, file: string): string {
  const relative = path.relative(from, file).split(path.sep).join('/')
  return relative.startsWith('.') ? relative : `./${relative}`
}

async function writeIfChanged(file: string, contents: string): Promise<boolean> {
  try {
    if ((await readFile(file, 'utf8')) === contents) return false
  } catch (error) {
    if (!isMissingFile(error)) throw error
  }
  await mkdir(path.dirname(file), { recursive: true })
  const temporary = path.join(
    path.dirname(file),
    `.${path.basename(file)}.${process.pid}.${temporaryFileId++}.tmp`,
  )
  let handle: FileHandle | undefined
  try {
    handle = await open(temporary, 'wx')
    await handle.writeFile(contents)
    await handle.sync()
    await handle.close()
    handle = undefined
    await rename(temporary, file)
  } catch (error) {
    await handle?.close()
    await rm(temporary, { force: true })
    throw error
  }
  return true
}

function compareRecords(left: IDocumentMetadata, right: IDocumentMetadata): number {
  return left.collection.localeCompare(right.collection) || left.key.localeCompare(right.key)
}

function documentMetadata(record: IDocumentRecord): IDocumentMetadata {
  return {
    cacheKey: record.cacheKey,
    cached: record.cached,
    collection: record.collection,
    dependencies: record.dependencies,
    derived: record.derived,
    diagnostics: record.diagnostics,
    file: record.file,
    frontmatter: record.frontmatter,
    hash: record.hash,
    key: record.key,
    locale: record.locale,
    modifiedAt: record.modifiedAt,
    projections: record.projections,
    slug: record.slug,
  }
}

function cleanFileId(file: string): string {
  const absolute = path.resolve(file.split('?')[0] ?? file)
  const missing: string[] = []
  let existing = absolute
  while (true) {
    try {
      return path.join(realpathSync.native(existing), ...missing)
    } catch (error) {
      if (!isMissingFile(error)) return absolute
      const parent = path.dirname(existing)
      if (parent === existing) return absolute
      missing.unshift(path.basename(existing))
      existing = parent
    }
  }
}

async function gitLastModified(
  root: string,
  files: string[],
  pathspecs: string[],
): Promise<Map<string, string>> {
  const wanted = new Set(files.map(cleanFileId))
  const relativePathspecs = pathspecs
    .map((value) => path.relative(root, value))
    .filter((value) => !value.startsWith('..') && !path.isAbsolute(value))
  if (wanted.size === 0 || relativePathspecs.length === 0) return new Map()
  try {
    const { stdout } = await execFileAsync(
      'git',
      [
        '-c',
        'core.quotepath=false',
        'log',
        '--format=AMAMO_COMMIT:%cI',
        '--name-only',
        '--relative',
        '--',
        ...relativePathspecs,
      ],
      { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
    )
    const modified = new Map<string, string>()
    let commitTime: string | undefined
    for (const rawLine of stdout.split('\n')) {
      const line = rawLine.endsWith('\r') ? rawLine.slice(0, -1) : rawLine
      if (line.startsWith('AMAMO_COMMIT:')) {
        commitTime = line.slice('AMAMO_COMMIT:'.length)
      } else if (line && commitTime) {
        const file = cleanFileId(path.resolve(root, line))
        if (wanted.has(file) && !modified.has(file)) modified.set(file, commitTime)
      }
    }
    return modified
  } catch {
    return new Map()
  }
}

function isMissingFile(error: unknown): boolean {
  return error instanceof Error && 'code' in error && error.code === 'ENOENT'
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error))
}
