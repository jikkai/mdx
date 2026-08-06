import { fileURLToPath } from 'node:url'

import { withAmamoMdx } from '../../dist/next.js'

const root = fileURLToPath(new URL('.', import.meta.url))

export default withAmamoMdx({
  root,
  collections: {
    posts: {
      directory: 'content/posts',
      schema: {
        type: 'object',
        properties: {
          title: { type: 'string' },
        },
        required: ['title'],
      },
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
