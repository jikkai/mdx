import type { ReactNode } from 'react'

export interface ITocItem {
  depth: number
  title: ReactNode
  url: string
}

export interface IStructuredDataHeading {
  content: string
  id: string
}

export interface IStructuredDataContent {
  content: string
  heading?: string
}

export interface IStructuredData {
  contents: IStructuredDataContent[]
  headings: IStructuredDataHeading[]
}
