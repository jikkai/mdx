import type { IAmamoMDXConfig } from './config.js'
import type { IDocumentRecord, IHighlightedCodeBlock, INativeDocumentInput } from './native.js'
import type { IShikiRenderer } from './shiki.js'
import { configurationFingerprint, prepareNativeBatch } from './native.js'
import { createShikiRenderer } from './shiki.js'

const renderers = new Map<string, Promise<IShikiRenderer>>()

export async function compileNextDocument(
  config: IAmamoMDXConfig,
  input: INativeDocumentInput,
): Promise<IDocumentRecord> {
  const batch = prepareNativeBatch(config, [input])
  let highlights: IHighlightedCodeBlock[] = []
  if (config.highlight.enabled && batch.codeBlocks.length > 0) {
    const fingerprint = configurationFingerprint(config)
    let renderer = renderers.get(fingerprint)
    if (!renderer) {
      renderer = createShikiRenderer(config.highlight)
      renderers.set(fingerprint, renderer)
    }
    highlights = await (await renderer).highlight(batch.codeBlocks)
  }
  const record = batch.finish(highlights)[0]
  if (!record) throw new Error(`AMAMO_NATIVE_RESULT_MISSING: ${input.file}`)
  return record
}
