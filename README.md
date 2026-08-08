# @amamo/mdx

Compile trusted MDX into JavaScript modules for a configurable JSX runtime (React by default),
collection metadata, and JSON manifests. A Rust native binding handles parsing, validation, media
rewriting, manifest projection, and persistent cache records; Shiki runs in JavaScript and feeds
highlighted HAST back into the same compile pipeline.

The package exposes three import paths:

| Import            | Purpose                                            |
| ----------------- | -------------------------------------------------- |
| `@amamo/mdx`      | Configure and drive the compiler directly.         |
| `@amamo/mdx/vite` | Compile MDX through Vite 8.                        |
| `@amamo/mdx/next` | Compile MDX for Next 16 with Turbopack or Webpack. |

## Quick start

```sh
pnpm add @amamo/mdx
```

`@amamo/mdx` requires Node.js 20.19 or newer and a [supported native
target](https://jikkai.github.io/mdx/native-targets/). It has no JavaScript or WASI fallback.

Create a serializable config:

```js
// amamo.config.mjs
import { defineConfig } from '@amamo/mdx'

export default defineConfig({
  root: import.meta.dirname,
  collections: {
    posts: {
      directory: 'content/posts',
      schema: {
        $schema: 'https://json-schema.org/draft/2020-12/schema',
        type: 'object',
        properties: { title: { type: 'string' } },
        required: ['title'],
      },
    },
  },
})
```

Then choose the integration that owns the build.

### Vite

```ts
// vite.config.ts
import { amamoMdx } from '@amamo/mdx/vite'
import { defineConfig } from 'vite'

import amamo from './amamo.config.mjs'

export default defineConfig({ plugins: [amamoMdx(amamo)] })
```

### Next

```ts
// next.config.ts
import { withAmamoMdx } from '@amamo/mdx/next'

import amamo from './amamo.config.mjs'

export default withAmamoMdx(amamo)({ reactStrictMode: true })
```

### Direct compiler API

```ts
import { createCompiler } from '@amamo/mdx'

import amamo from './amamo.config.mjs'

const compiler = await createCompiler(amamo)
try {
  const result = await compiler.build()
  console.log(result)
} finally {
  await compiler.dispose()
}
```

The first build writes these compiler-owned files under `generatedDirectory` (default
`.amamo-mdx`):

- `collections.mjs` — collection metadata with lazy imports of the source MDX files.
- `collections.d.ts` — a companion declaration output for the collection registry.
- `index.json` — the private index used by the Next loader.

Cache and manifest paths are configured separately and are resolved from `root`.

## Security boundary

MDX modules can execute JavaScript when the host imports or renders them. Compile only content from
trusted authors; schema validation is not a sandbox. Mark top-level frontmatter fields as
`sensitive` to keep their plaintext out of compiled modules, cache records, and manifests. See the
[security model](https://jikkai.github.io/mdx/security/) before handling secrets.

## Documentation

The complete English and Simplified Chinese documentation covers:

- [Getting started](https://jikkai.github.io/mdx/getting-started/)
- [Configuration reference](https://jikkai.github.io/mdx/configuration/)
- [Compiler API](https://jikkai.github.io/mdx/compiler-api/)
- [Vite integration](https://jikkai.github.io/mdx/vite/)
- [Next integration](https://jikkai.github.io/mdx/next/)
- [Native targets](https://jikkai.github.io/mdx/native-targets/)

See the [contributing guide](https://github.com/jikkai/mdx/blob/main/CONTRIBUTING.md) to build and
verify the repository locally.

## License

MIT
