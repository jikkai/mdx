# @amamo/mdx

An independent MDX content compiler with a Rust core, official Shiki highlighting, JSON Schema validation, media imports, deterministic manifests, and content-addressed caching. The same compiler state powers the root API, Vite 8, and Next 16.

## Install

```sh
pnpm add @amamo/mdx
```

Node.js 20.19 or newer is required. A supported native package is installed through `optionalDependencies`; there is no JavaScript or WASI fallback.

## Configure

Configuration is plain serializable data. Functions and executable plugins are intentionally rejected.

```ts
// amamo.config.ts
import { defineConfig } from '@amamo/mdx'

export default defineConfig({
  root: import.meta.dirname,
  collections: {
    posts: {
      directory: 'content/posts',
      schema: {
        $schema: 'https://json-schema.org/draft/2020-12/schema',
        type: 'object',
        properties: {
          title: { type: 'string' },
          password: { type: 'string' },
        },
        required: ['title'],
      },
      sensitive: ['password'],
    },
  },
  highlight: {
    provider: 'shiki',
    engine: 'oniguruma',
    themes: { light: 'vitesse-light', dark: 'vitesse-dark' },
    unknownLanguage: 'plain',
  },
  media: { missing: 'warn' },
  manifests: {
    public: {
      output: '.amamo-mdx/public.json',
      fields: { key: 'key', title: 'title' },
    },
    server: {
      output: '.amamo-mdx/server.json',
      key: 'key',
      fields: {
        key: 'key',
        protected: { from: 'password', transform: 'exists' },
        passwordHash: { from: 'password', transform: 'sha256' },
      },
    },
  },
})
```

The default highlighter keeps official Shiki in JavaScript and uses its Oniguruma WASM engine. Set `engine: 'javascript'` when avoiding Oniguruma is more important than grammar compatibility. Relative Markdown media URLs become static imports; authored JSX is left untouched. `unknownLanguage` and `media.missing` accept `error` or their permissive `plain`/`warn` policies.

## Compiler API

```ts
import { createCompiler } from '@amamo/mdx'
import config from './amamo.config.js'

const compiler = await createCompiler(config)
await compiler.build()
await compiler.transform('content/posts/hello.mdx')
await compiler.remove('content/posts/deleted.mdx')
await compiler.dispose()
```

`build()` produces `.amamo-mdx/collections.mjs`, its declaration file, the private loader index, and configured manifests. Unchanged bytes are not rewritten.

## Vite 8

```ts
import { defineConfig } from 'vite'
import { amamoMdx } from '@amamo/mdx/vite'
import amamo from './amamo.config.js'

export default defineConfig({ plugins: [amamoMdx(amamo)] })
```

The adapter joins Vite environments onto one startup build and uses Vite's existing watcher for add, change, and delete events.

## Next 16

```js
// next.config.mjs
import { withAmamoMdx } from '@amamo/mdx/next'
import amamo from './amamo.config.js'

export default withAmamoMdx(amamo)({ reactStrictMode: true })
```

The same private read-only loader is registered for default Turbopack and opt-in Webpack. The config wrapper also accepts and awaits an existing Next config function.

## Security

MDX can contain executable JavaScript. Compile only content from trusted authors. Schema validation is not a sandbox. Fields marked `sensitive` are removed before modules, cache records, and public frontmatter are serialized; manifests may only inspect them with `exists` or hash them with `sha256`.

Relative media imports are confined to the configured root, including symlink resolution. Cache and generated output writes use same-directory temporary files and atomic rename.

## Native targets

- macOS arm64 and x64
- Linux arm64 and x64, glibc and musl
- Windows x64 MSVC

Unsupported targets fail during native binding load. No WASI or JavaScript fallback is shipped.

## License

MIT
