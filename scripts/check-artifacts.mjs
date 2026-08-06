import { createRequire } from 'node:module'
import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import process from 'node:process'

const targets = [
  'darwin-arm64',
  'darwin-x64',
  'linux-arm64-gnu',
  'linux-arm64-musl',
  'linux-x64-gnu',
  'linux-x64-musl',
  'win32-x64-msvc',
]

if (process.argv[2] === '--local') {
  const loader = path.resolve(process.argv[3] ?? 'native.cjs')
  createRequire(import.meta.url)(loader)
  const target = localTarget()
  await readFile(path.join(path.dirname(loader), `index.${target}.node`))
  console.log(JSON.stringify({ loader, target }))
} else {
  const rootPackage = JSON.parse(
    await readFile(new URL('../package.json', import.meta.url), 'utf8'),
  )
  const configuredRepository = rootPackage.repository?.url
  const expectedRepository = process.env.GITHUB_REPOSITORY
    ? `${process.env.GITHUB_SERVER_URL ?? 'https://github.com'}/${process.env.GITHUB_REPOSITORY}`
    : configuredRepository
  if (!configuredRepository || configuredRepository !== expectedRepository) {
    throw new Error(`Expected root repository.url to be ${expectedRepository}`)
  }

  const artifactDirectory = path.resolve(process.argv[2] ?? 'artifacts')
  const directories = (await readdir(artifactDirectory, { withFileTypes: true }))
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .toSorted()
  if (JSON.stringify(directories) !== JSON.stringify(targets)) {
    throw new Error(`Expected only ${targets.join(', ')} in ${artifactDirectory}`)
  }

  await Promise.all(
    targets.map(async (target) => {
      const directory = path.join(artifactDirectory, target)
      const [packageSource, files] = await Promise.all([
        readFile(path.join(directory, 'package.json'), 'utf8'),
        readdir(directory),
      ])
      const pkg = JSON.parse(packageSource)
      const expectedBinding = `index.${target}.node`
      const bindings = files.filter((file) => file.endsWith('.node'))
      if (
        pkg.name !== `@amamo/mdx-${target}` ||
        pkg.repository?.url !== expectedRepository ||
        pkg.main !== expectedBinding ||
        bindings.length !== 1 ||
        bindings[0] !== expectedBinding
      ) {
        throw new Error(`Invalid native package for ${target}`)
      }
    }),
  )
  console.log(JSON.stringify({ directory: artifactDirectory, targets }))
}

function localTarget() {
  if (process.platform === 'darwin' && ['arm64', 'x64'].includes(process.arch)) {
    return `darwin-${process.arch}`
  }
  if (process.platform === 'win32' && process.arch === 'x64') return 'win32-x64-msvc'
  if (process.platform === 'linux' && ['arm64', 'x64'].includes(process.arch)) {
    const report = process.report?.getReport()
    const libc = report?.header?.glibcVersionRuntime ? 'gnu' : 'musl'
    return `linux-${process.arch}-${libc}`
  }
  throw new Error(`Unsupported local target: ${process.platform}-${process.arch}`)
}
