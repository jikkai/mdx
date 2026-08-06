import { createHighlighterCore } from 'shiki/core'
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript'
import { createOnigurumaEngine } from 'shiki/engine/oniguruma'
import { bundledLanguages, bundledLanguagesAlias } from 'shiki/langs'
import { bundledThemes } from 'shiki/themes'

import type { INormalizedHighlightConfig, JsonValue } from './config.js'
import type { ICodeBlock, IHighlightedCodeBlock } from './native.js'

export interface IShikiRenderer {
  dispose(): void
  highlight(blocks: ICodeBlock[]): Promise<IHighlightedCodeBlock[]>
}

function themeLoader(name: string) {
  if (!Object.hasOwn(bundledThemes, name)) {
    throw new Error(`AMAMO_SHIKI_UNKNOWN_THEME: Unknown bundled Shiki theme \`${name}\``)
  }
  return bundledThemes[name as keyof typeof bundledThemes]
}

function languageLoader(name: string) {
  if (Object.hasOwn(bundledLanguages, name)) {
    return bundledLanguages[name as keyof typeof bundledLanguages]
  }
  if (Object.hasOwn(bundledLanguagesAlias, name)) {
    return bundledLanguagesAlias[name as keyof typeof bundledLanguagesAlias]
  }
  return undefined
}

export async function createShikiRenderer(
  config: INormalizedHighlightConfig,
): Promise<IShikiRenderer> {
  const engine =
    config.engine === 'javascript'
      ? createJavaScriptRegexEngine()
      : createOnigurumaEngine(import('shiki/wasm'))
  const highlighter = await createHighlighterCore({
    engine,
    langs: [],
    themes: [themeLoader(config.themes.light), themeLoader(config.themes.dark)],
  })
  const loading = new Map<string, Promise<string>>()
  let disposed = false

  async function loadLanguage(requested: string | null): Promise<string> {
    if (!requested || requested === 'text' || requested === 'plain' || requested === 'plaintext') {
      return 'text'
    }
    const existing = loading.get(requested)
    if (existing) return existing

    const loader = languageLoader(requested)
    if (!loader) {
      if (config.unknownLanguage === 'plain') return 'text'
      throw new Error(
        `AMAMO_SHIKI_UNKNOWN_LANGUAGE: Unknown bundled Shiki language \`${requested}\``,
      )
    }
    const promise = highlighter.loadLanguage(loader).then(() => requested)
    loading.set(requested, promise)
    return promise
  }

  if (config.languages !== 'auto') {
    await Promise.all(config.languages.map((language) => loadLanguage(language)))
  }

  return {
    dispose() {
      if (disposed) return
      disposed = true
      highlighter.dispose()
    },
    async highlight(blocks) {
      if (disposed) throw new Error('AMAMO_SHIKI_DISPOSED: Shiki renderer is disposed')
      const languages = await Promise.all(blocks.map((block) => loadLanguage(block.lang)))
      return blocks.map((block, index) => {
        const hast = highlighter.codeToHast(block.code, {
          colorReplacements: config.colorReplacements,
          defaultColor: false,
          lang: languages[index] ?? 'text',
          meta: block.meta ? { __raw: block.meta } : undefined,
          themes: {
            dark: config.themes.dark,
            light: config.themes.light,
          },
        })
        return {
          blockId: block.blockId,
          documentId: block.documentId,
          hast: JSON.parse(JSON.stringify(hast)) as JsonValue,
        }
      })
    },
  }
}
