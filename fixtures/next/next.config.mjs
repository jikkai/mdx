import { fileURLToPath } from 'node:url'

import { z } from '../../dist/index.js'
import { withAmamoMdx } from '../../dist/next.js'

const root = fileURLToPath(new URL('.', import.meta.url))

export default withAmamoMdx({
  root,
  collections: {
    posts: {
      directory: 'content/posts',
      schema: z.object({ title: z.string() }),
    },
  },
  manifests: {
    public: {
      output: '.amamo-mdx/public.json',
      fields: {
        key: 'key',
        title: 'title',
      },
    },
  },
})({
  output: 'export',
  reactStrictMode: true,
})
