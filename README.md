# @amamo/mdx

Build MDX collections for Vite 8, Next 16, or a custom Node.js build. Define each collection with
the package's `z` schema builder, then import MDX as application modules or consume the generated
collection registry and JSON manifests.

`@amamo/mdx` provides:

- frontmatter validation and defaults;
- JavaScript modules for a configurable JSX runtime, with React as the default;
- fenced-code highlighting and Markdown media imports;
- collection metadata with companion TypeScript declarations;
- configurable JSON manifests and a persistent build cache.

## Requirements

- Node.js 20.19 or newer.
- A [supported native target](https://jikkai.github.io/mdx/native-targets/). There is no JavaScript
  or WASI fallback for MDX compilation.
- React 19 when using the default JSX runtime.

MDX can contain imports, expressions, and JSX. Compile content from authors who are allowed to add
application code.

## Install

```sh
pnpm add @amamo/mdx
```

The package manager installs the platform package for the current operating system and CPU. Install
dependencies again after moving the project to a different platform instead of copying
`node_modules`.

## Define a collection

Create `amamo.config.mjs`:

```js
import { defineConfig, z } from '@amamo/mdx'

export default defineConfig({
  root: import.meta.dirname,
  collections: {
    posts: {
      directory: 'content/posts',
      schema: z.object({
        title: z.string(),
        publishedAt: z.string().optional(),
      }),
    },
  },
})
```

Then add `content/posts/hello.mdx`:

```mdx
---
title: Hello
publishedAt: 2026-08-15
---

# Hello

This document is compiled by @amamo/mdx.
```

The collection directory must exist before the first full build. Relative collection, cache,
generated, and manifest paths are resolved from `root`.

## Choose an integration

### Vite

```ts
// vite.config.ts
import { amamoMdx } from '@amamo/mdx/vite'
import { defineConfig } from 'vite'

import amamo from './amamo.config.mjs'

export default defineConfig({
  plugins: [amamoMdx(amamo)],
})
```

### Next

```ts
// next.config.ts
import { withAmamoMdx } from '@amamo/mdx/next'

import amamo from './amamo.config.mjs'

export default withAmamoMdx(amamo)({
  reactStrictMode: true,
})
```

### Direct compiler API

```js
// build-content.mjs
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

Run the script with `node build-content.mjs`.

## Use compiled content

With the Vite plugin or Next wrapper configured, import an MDX file like an application module:

```tsx
import Post, { frontmatter } from './content/posts/hello.mdx'

export function Page() {
  return (
    <main>
      <h1>{frontmatter.title}</h1>
      <Post />
    </main>
  )
}
```

Or load a document from the generated registry:

```ts
import { collections } from './.amamo-mdx/collections.mjs'

const hello = collections.posts.find((document) => document.slug === 'hello')
const module = await hello?.load()
```

Registry `load()` functions import the source MDX file, so they must run through the configured Vite
plugin or Next loader.

## Generated files

The first build writes these files under `generatedDirectory`, which defaults to `.amamo-mdx`:

- `collections.mjs` — sorted collection metadata and lazy source imports;
- `collections.d.ts` — TypeScript declarations for the registry;
- `index.json` — the source-to-cache index used by the Next loader.

Cache and manifest paths are configured separately from `generatedDirectory`. Add `.amamo-mdx/` to
the host repository's ignore file unless the application deliberately tracks generated output.

## Package entry points

| Import            | Use it for                                 |
| ----------------- | ------------------------------------------ |
| `@amamo/mdx`      | Configuration and the direct compiler API. |
| `@amamo/mdx/vite` | Vite 8 development and production builds.  |
| `@amamo/mdx/next` | Next 16 development and production builds. |

## Documentation

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
