import { invoke } from '@tauri-apps/api/core'

export type Os = 'linux' | 'windows' | 'macos'

export interface DeviceView {
  id: string
  short: string
  name: string
  os: Os
  connected: boolean
  address: string | null
  appliedRevision: number | null
  appliedAt: number | null
}

export interface EditView {
  path: string
  before: string | null
  after: string | null
}

export interface View {
  stage: 'setup' | 'rules' | 'ready'
  message: string | null
  root: string | null
  me: DeviceView | null
  other: DeviceView | null
  sharedRevision: number | null
  rootMode: string | null
  edits: EditView[]
  conflicts: string[]
  localEntries: number
  remoteEntries: number | null
  remoteListAge: number | null
  alwaysIgnore: string[]
}

export interface Side {
  size: number
  files: number
}

export interface NodeView {
  name: string
  path: string
  dir: boolean
  here: Side | null
  there: Side | null
  explicit: string | null
  effective: string
  from: string | null
}

export interface Problem {
  path: string
  message: string
}

export type Action =
  | { kind: 'remove'; id: string; rel: string; path: string }
  | { kind: 'update'; id: string; rel: string; path: string; changes: string[]; folderType: string }
  | { kind: 'add'; id: string; rel: string; path: string; folderType: string; filesOnly: boolean }

export interface Plan {
  revision: number
  actions: Action[]
  problems: Problem[]
  warnings: string[]
  unchanged: number
  folders: number
}

export interface PlanView {
  plan: Plan
  edits: EditView[]
}

export interface Report {
  added: number
  updated: number
  removed: number
}

export const api = {
  state: () => invoke<View>('state'),
  refresh: () => invoke<View>('refresh'),
  children: (path: string) => invoke<NodeView[]>('children', { path }),
  setRule: (path: string, dir: boolean, mode: string | null) => invoke<View>('set_rule', { path, dir, mode }),
  discard: () => invoke<View>('discard'),
  plan: () => invoke<PlanView>('plan'),
  apply: () => invoke<Report>('apply'),
  init: (root: string) => invoke<View>('init', { root }),
  createRules: (otherOs: Os) => invoke<View>('create_rules', { otherOs }),
}
