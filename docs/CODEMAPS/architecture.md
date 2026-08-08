<!-- Generated: 2026-08-08 | Files scanned: 43 | Token estimate: ~850 -->

# Architecture codemap

## Package surfaces

| Import            | Definition     | Runtime owner                 |
| ----------------- | -------------- | ----------------------------- |
| `@amamo/mdx`      | `src/index.ts` | Caller-created compiler       |
| `@amamo/mdx/vite` | `src/vite.ts`  | One compiler per Vite plugin  |
| `@amamo/mdx/next` | `src/next.ts`  | One compiler per Next wrapper |

The package is ESM and import-only. It has no CLI or JavaScript fallback.

## Compile flow

```text
IAmamoMdxConfig
  -> src/config.ts: normalizeConfig
  -> src/compiler.ts: discover + read + serialize operations
  -> src/native.ts: prepareNativeBatch
  -> native/src/: YAML -> schema -> MDAST/HAST -> media + projections
  -> src/shiki.ts: highlighted HAST
  -> native/src/: inject highlights -> JS module -> cache record
  -> src/compiler.ts: manifests + collections.mjs + collections.d.ts + index.json
```

The Rust binding owns parsing, validation, MDX compilation, manifest projection, and persistent
cache records. Shiki stays in JavaScript so it can use the official bundled themes, grammars, and
engines.

## Adapter paths

```text
Vite buildStart -> compiler.build
Vite transform  -> compiler.transform -> returned JavaScript
Vite watcher    -> transform/remove -> generated module invalidation -> full reload

Next dev/build config -> compiler.build -> index.json + cache records
Other Next phases     -> loader rules only
Next dev              -> recursive compiler watcher
Turbopack/Webpack     -> next-loader.cjs -> verify index fingerprint -> read cached module
```

Vite does not read `index.json`. Next requires the persistent cache and only registers `*.mdx`
rules, even if a collection config lists another extension.

## Persistence

- `generatedDirectory`: `collections.mjs`, `collections.d.ts`, `index.json`.
- `cache.directory`: BLAKE3-addressed document records, including highlighted output.
- `manifests.*.output`: JSON arrays or keyed objects resolved independently from `root`.
- Shared generated files and manifests compare bytes before writing.
- Cache records and shared outputs use same-directory temporary files plus rename.

## Key files

| Path                     | Role                                                          |
| ------------------------ | ------------------------------------------------------------- |
| `src/config.ts`          | Public configuration contract and defaults                    |
| `src/compiler.ts`        | Compiler state, discovery, queue, watchers, output generation |
| `src/native.ts`          | N-API loading and error mapping                               |
| `src/shiki.ts`           | Shiki lifecycle and in-memory language loading                |
| `native/src/document.rs` | Frontmatter, MDX tree, derived metadata, module emission      |
| `native/src/hast.rs`     | Highlight injection and Markdown media imports                |
| `native/src/cache.rs`    | Cache keys, atomic records, corruption recovery, pruning      |
| `native/src/manifest.rs` | Projection, sorting, keyed output, JSON rendering             |
| `src/__tests__/`         | Real binding and adapter regression coverage                  |

## Verification path

`pnpm run check` -> format -> Oxlint -> Clippy -> Rust tests -> TypeScript typecheck -> native/TS
build + Vitest -> docs typecheck/build. CI then builds the Next fixture with both Turbopack and
Webpack and dry-runs the root npm package.
