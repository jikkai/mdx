import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'

import type { IAmamoMdxConfig } from '../config.js'
import { z } from '../config.js'

export interface ICompilerFixture {
  cleanup(): Promise<void>
  collectionModule: string
  config: IAmamoMdxConfig
  generatedDirectory: string
  post: string
  publicManifest: string
  root: string
  serverManifest: string
  source: string
  updatedSource: string
}

export async function createCompilerFixture(): Promise<ICompilerFixture> {
  const root = await mkdtemp(path.join(tmpdir(), 'amamo-mdx-compiler-'))
  const content = path.join(root, 'content/posts')
  const generatedDirectory = path.join(root, '.amamo-mdx')
  const post = path.join(content, 'hello.mdx')
  const source = `---
title: Hello
category: article
---
# Hello

![cover](./image.png)

\`\`\`ts
const answer: number = 42
\`\`\`
`
  const updatedSource = source.replaceAll('Hello', 'Updated')
  await mkdir(content, { recursive: true })
  await writeFile(path.join(content, 'image.png'), 'image')
  await writeFile(post, source)

  return {
    async cleanup() {
      await rm(root, { recursive: true, force: true })
    },
    collectionModule: path.join(generatedDirectory, 'collections.mjs'),
    config: {
      root,
      collections: {
        posts: {
          directory: 'content/posts',
          schema: z.object({
            title: z.string(),
            category: z.string().optional(),
          }),
        },
      },
      derived: { readingTime: true },
      manifests: {
        public: {
          output: '.amamo-mdx/public.json',
          fields: {
            key: 'key',
            title: 'title',
            readingTime: 'derived.readingTime',
          },
          sort: [{ field: 'title' }],
        },
        server: {
          output: '.amamo-mdx/server.json',
          key: 'key',
          fields: {
            key: 'key',
            title: 'title',
            category: 'category',
            categorized: { from: 'category', transform: 'exists' },
            categoryHash: { from: 'category', transform: 'sha256' },
          },
        },
      },
    },
    generatedDirectory,
    post,
    publicManifest: path.join(generatedDirectory, 'public.json'),
    root,
    serverManifest: path.join(generatedDirectory, 'server.json'),
    source,
    updatedSource,
  }
}
