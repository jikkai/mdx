import Post, { frontmatter } from '../content/posts/hello.mdx'

export default function Page() {
  return (
    <main>
      <p>{frontmatter.title}</p>
      <Post />
    </main>
  )
}
