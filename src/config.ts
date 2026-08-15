import { Buffer } from 'node:buffer'
import { existsSync, realpathSync } from 'node:fs'
import path from 'node:path'

import * as z from 'zod'

export { z }

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue }

export type ManifestField =
  | string
  | {
      default?: JsonValue
      from: string
      transform?: 'exists' | 'mediaUrl' | 'sha256'
    }

export interface ILocaleConfig {
  default: string
  names: string[]
}

export interface ISlugConfig {
  indexNames?: string[]
}

export interface IFrontmatterSchema {
  readonly shape: Readonly<Record<string, unknown>>
  toJSONSchema(): unknown
}

export interface ICollectionConfig {
  directory: string
  extensions?: string[]
  locales?: ILocaleConfig
  schema: IFrontmatterSchema
  slug?: ISlugConfig
}

export interface IMathConfig {
  macros?: Record<string, string>
  singleDollar?: boolean
}

export interface IMdxExtensionsConfig {
  footnotes?: boolean
  headingIds?: boolean
  taskLists?: boolean
}

export interface IMdxConfig {
  extensions?: IMdxExtensionsConfig
  gfm?: boolean
  hardBreaks?: boolean
  jsxImportSource?: string
  math?: false | IMathConfig
  providerImportSource?: string
}

export interface IHighlightConfig {
  colorReplacements?: Record<string, string>
  engine?: 'javascript' | 'oniguruma'
  languages?: 'auto' | string[]
  provider: 'shiki'
  themes: {
    dark: string
    light: string
  }
  unknownLanguage?: 'error' | 'plain'
}

export interface IMediaConfig {
  attributes?: Record<string, string[]>
  missing?: 'error' | 'warn'
}

export interface IDerivedConfig {
  lastModified?: boolean
  readingTime?: boolean
}

export interface IManifestConfig {
  collections?: string[]
  fields: Record<string, ManifestField>
  key?: string
  output: string
  sort?: Array<{
    direction?: 'asc' | 'desc'
    field: string
  }>
}

export interface ICacheConfig {
  directory?: string
}

export interface IAmamoMdxConfig {
  cache?: false | ICacheConfig
  collections: Record<string, ICollectionConfig>
  derived?: IDerivedConfig
  generatedDirectory?: string
  highlight?: false | IHighlightConfig
  manifests?: Record<string, IManifestConfig>
  mdx?: IMdxConfig
  media?: false | IMediaConfig
  root?: string
}

export interface INormalizedCollectionConfig {
  directory: string
  extensions: string[]
  locales?: ILocaleConfig
  schema: { [key: string]: JsonValue }
  slug: {
    indexNames: string[]
  }
}

export interface INormalizedHighlightConfig {
  colorReplacements: Record<string, string>
  enabled: boolean
  engine: 'javascript' | 'oniguruma'
  languages: 'auto' | string[]
  provider: 'shiki'
  themes: {
    dark: string
    light: string
  }
  unknownLanguage: 'error' | 'plain'
}

export interface INormalizedMediaConfig {
  attributes: Record<string, string[]>
  enabled: boolean
  missing: 'error' | 'warn'
}

export interface INormalizedManifestConfig extends Omit<IManifestConfig, 'collections' | 'output'> {
  collections: string[]
  output: string
}

export interface IAmamoMDXConfig {
  cache: {
    directory: string
    enabled: boolean
  }
  collections: Record<string, INormalizedCollectionConfig>
  derived: Required<IDerivedConfig>
  generatedDirectory: string
  highlight: INormalizedHighlightConfig
  manifests: Record<string, INormalizedManifestConfig>
  mdx: {
    extensions: Required<IMdxExtensionsConfig>
    gfm: boolean
    hardBreaks: boolean
    jsxImportSource: string
    math: {
      enabled: boolean
      macros: Record<string, string>
      singleDollar: boolean
    }
    providerImportSource: string
  }
  media: INormalizedMediaConfig
  root: string
}

const DEFAULT_MEDIA_ATTRIBUTES: Record<string, string[]> = {
  audio: ['src'],
  embed: ['src'],
  img: ['src', 'srcset'],
  object: ['data'],
  source: ['src', 'srcset'],
  track: ['src'],
  video: ['src', 'poster'],
}
const MATH_MACRO_NAME = /^\\(?:[A-Za-z@]+|[^\s])$/u
const MAX_MATH_MACRO_BYTES = 1024
const MAX_MATH_MACROS_BYTES = 16 * 1024

function notSerializable(location: string, reason: string): never {
  throw new TypeError(`AMAMO_CONFIG_NOT_SERIALIZABLE: ${location} ${reason}`)
}

function assertPlainData(
  value: unknown,
  location: string,
  active: WeakSet<object>,
  segments: string[] = [],
): void {
  if (segments.length === 3 && segments[0] === 'collections' && segments[2] === 'schema') return
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) notSerializable(location, 'must be a finite number')
    return
  }
  if (typeof value !== 'object') notSerializable(location, `contains ${typeof value}`)
  if (active.has(value)) notSerializable(location, 'contains a cycle')

  const prototype = Object.getPrototypeOf(value)
  if (!Array.isArray(value) && prototype !== Object.prototype && prototype !== null) {
    notSerializable(location, 'must be a plain object')
  }

  active.add(value)
  for (const key of Reflect.ownKeys(value)) {
    if (typeof key === 'symbol') notSerializable(location, 'contains a symbol key')
    const descriptor = Object.getOwnPropertyDescriptor(value, key)
    if (!descriptor || !('value' in descriptor))
      notSerializable(`${location}.${key}`, 'contains an accessor')
    assertPlainData(descriptor.value, `${location}.${key}`, active, [...segments, key])
  }
  active.delete(value)
}

function requireNonEmpty(value: string, location: string): string {
  if (value.length === 0) throw new TypeError(`AMAMO_CONFIG_INVALID: ${location} must not be empty`)
  return value
}

function normalizeCollection(
  root: string,
  name: string,
  config: ICollectionConfig,
): INormalizedCollectionConfig {
  const extensions = config.extensions ?? ['.mdx']
  if (extensions.length === 0 || extensions.some((extension) => !extension.startsWith('.'))) {
    throw new TypeError(
      `AMAMO_CONFIG_INVALID: collections.${name}.extensions must contain dotted extensions`,
    )
  }
  if (config.locales && !config.locales.names.includes(config.locales.default)) {
    throw new TypeError(
      `AMAMO_CONFIG_INVALID: collections.${name}.locales.default must be listed in names`,
    )
  }

  const directory = path.resolve(
    root,
    requireNonEmpty(config.directory, `collections.${name}.directory`),
  )
  if (
    typeof config.schema?.shape !== 'object' ||
    config.schema.shape === null ||
    typeof config.schema.toJSONSchema !== 'function'
  ) {
    throw new TypeError(
      `AMAMO_CONFIG_INVALID: collections.${name}.schema must be a compatible object schema`,
    )
  }
  let schema: { [key: string]: JsonValue }
  try {
    schema = JSON.parse(JSON.stringify(config.schema.toJSONSchema())) as {
      [key: string]: JsonValue
    }
  } catch (error) {
    const reason = error instanceof Error ? error.message : 'could not be converted to JSON Schema'
    throw new TypeError(`AMAMO_CONFIG_INVALID: collections.${name}.schema ${reason}`, {
      cause: error,
    })
  }
  if (schema.type !== 'object') {
    throw new TypeError(
      `AMAMO_CONFIG_INVALID: collections.${name}.schema must convert to an object schema`,
    )
  }
  assertPlainData(schema, `collections.${name}.schema`, new WeakSet())

  return {
    directory: existsSync(directory) ? realpathSync.native(directory) : directory,
    extensions: [...extensions],
    locales: config.locales
      ? { default: config.locales.default, names: [...config.locales.names] }
      : undefined,
    schema,
    slug: { indexNames: [...(config.slug?.indexNames ?? ['index', 'page'])] },
  }
}

export function defineConfig<T extends IAmamoMdxConfig>(config: T): T {
  return config
}

export function normalizeConfig(config: IAmamoMdxConfig): IAmamoMDXConfig {
  assertPlainData(config, 'config', new WeakSet())
  if (Object.hasOwn(config, 'math')) {
    throw new TypeError('AMAMO_CONFIG_INVALID: math moved to mdx.math')
  }
  if (!config.collections || Object.keys(config.collections).length === 0) {
    throw new TypeError('AMAMO_CONFIG_INVALID: collections must not be empty')
  }

  const resolvedRoot = path.resolve(config.root ?? process.cwd())
  const root = existsSync(resolvedRoot) ? realpathSync.native(resolvedRoot) : resolvedRoot
  const generatedDirectory = path.resolve(root, config.generatedDirectory ?? '.amamo-mdx')
  const collectionNames = Object.keys(config.collections)
  const collections = Object.fromEntries(
    Object.entries(config.collections).map(([name, collection]) => [
      requireNonEmpty(name, 'collection name'),
      normalizeCollection(root, name, collection),
    ]),
  )
  const manifests = Object.fromEntries(
    Object.entries(config.manifests ?? {}).map(([name, manifest]) => [
      requireNonEmpty(name, 'manifest name'),
      {
        ...manifest,
        collections: [...(manifest.collections ?? collectionNames)],
        output: path.resolve(root, manifest.output),
      },
    ]),
  )

  const highlight = config.highlight === false ? undefined : config.highlight
  const math = config.mdx?.math === false ? undefined : config.mdx?.math
  const media = config.media === false ? undefined : config.media
  const gfm = config.mdx?.gfm ?? true

  let macroBytes = 0
  for (const [name, expansion] of Object.entries(math?.macros ?? {})) {
    if (!name.startsWith('\\')) {
      throw new TypeError(
        `AMAMO_CONFIG_INVALID: mdx.math.macros.${name} must start with a backslash`,
      )
    }
    if (!MATH_MACRO_NAME.test(name)) {
      throw new TypeError(
        `AMAMO_CONFIG_INVALID: mdx.math.macros.${name} must be one TeX control sequence`,
      )
    }
    const expansionBytes = Buffer.byteLength(expansion)
    if (expansionBytes > MAX_MATH_MACRO_BYTES) {
      throw new TypeError(
        `AMAMO_CONFIG_INVALID: mdx.math.macros.${name} exceeds the ${MAX_MATH_MACRO_BYTES}-byte limit`,
      )
    }
    macroBytes += Buffer.byteLength(name) + expansionBytes
    if (macroBytes > MAX_MATH_MACROS_BYTES) {
      throw new TypeError(
        `AMAMO_CONFIG_INVALID: mdx.math.macros exceeds the ${MAX_MATH_MACROS_BYTES}-byte total limit`,
      )
    }
  }

  return {
    cache: {
      directory: path.resolve(
        root,
        config.cache === false
          ? '.amamo-mdx/cache'
          : (config.cache?.directory ?? '.amamo-mdx/cache'),
      ),
      enabled: config.cache !== false,
    },
    collections,
    derived: {
      lastModified: config.derived?.lastModified ?? false,
      readingTime: config.derived?.readingTime ?? false,
    },
    generatedDirectory,
    highlight: {
      colorReplacements: { ...highlight?.colorReplacements },
      enabled: config.highlight !== false,
      engine: highlight?.engine ?? 'oniguruma',
      languages:
        highlight?.languages === 'auto' || highlight?.languages === undefined
          ? 'auto'
          : [...highlight.languages],
      provider: 'shiki',
      themes: highlight?.themes ?? { light: 'vitesse-light', dark: 'vitesse-dark' },
      unknownLanguage: highlight?.unknownLanguage ?? 'error',
    },
    manifests,
    mdx: {
      extensions: {
        footnotes: config.mdx?.extensions?.footnotes ?? gfm,
        headingIds: config.mdx?.extensions?.headingIds ?? false,
        taskLists: config.mdx?.extensions?.taskLists ?? gfm,
      },
      gfm,
      hardBreaks: config.mdx?.hardBreaks ?? false,
      jsxImportSource: config.mdx?.jsxImportSource ?? 'react',
      math: {
        enabled: config.mdx?.math !== undefined && config.mdx.math !== false,
        macros: { ...math?.macros },
        singleDollar: math?.singleDollar ?? true,
      },
      providerImportSource: config.mdx?.providerImportSource ?? '',
    },
    media: {
      attributes: Object.fromEntries(
        Object.entries(media?.attributes ?? DEFAULT_MEDIA_ATTRIBUTES).map(([tag, attributes]) => [
          tag,
          [...attributes],
        ]),
      ),
      enabled: config.media !== false,
      missing: media?.missing ?? 'error',
    },
    root,
  }
}
