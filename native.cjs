'use strict'

const target = nativeTarget()
const localBinding = `./index.${target}.node`
const packageName = `@amamo/mdx-${target}`

try {
  module.exports = require(localBinding)
} catch (localError) {
  try {
    const binding = require(packageName)
    const bindingVersion = require(`${packageName}/package.json`).version
    const packageVersion = require('./package.json').version
    if (bindingVersion !== packageVersion) {
      throw new Error(
        `AMAMO_NATIVE_VERSION_MISMATCH: expected ${packageVersion}, received ${bindingVersion}`,
      )
    }
    module.exports = binding
  } catch (packageError) {
    throw new Error(
      `AMAMO_NATIVE_UNAVAILABLE: No native binding for ${target}; reinstall @amamo/mdx on a supported platform`,
      { cause: new AggregateError([localError, packageError]) },
    )
  }
}

function nativeTarget() {
  if (process.platform === 'darwin' && ['arm64', 'x64'].includes(process.arch)) {
    return `darwin-${process.arch}`
  }
  if (process.platform === 'win32' && process.arch === 'x64') return 'win32-x64-msvc'
  if (process.platform === 'linux' && ['arm64', 'x64'].includes(process.arch)) {
    const report = process.report?.getReport()
    const libc = report?.header?.glibcVersionRuntime ? 'gnu' : 'musl'
    return `linux-${process.arch}-${libc}`
  }
  throw new Error(`AMAMO_NATIVE_UNSUPPORTED: ${process.platform}-${process.arch}`)
}
