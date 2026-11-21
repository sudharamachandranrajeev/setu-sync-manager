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

export interface HoldView {
  folder: string
  rel: string | null
  deletes: number
  at: number
}

export interface WatchView {
  running: boolean
  gateOpen: boolean
  reason: string | null
  metered: boolean
  relayed: boolean
  allowMetered: boolean
  allowRelay: boolean
  holds: HoldView[]
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
  fileConflicts: number
  localEntries: number
  remoteEntries: number | null
  remoteListAge: number | null
  alwaysIgnore: string[]
  leaveOut: LeaveOutEdits
  /** Paused here by the person: until this Unix time, or 0 until resumed. */
  pausedUntil: number | null
  watch: WatchView | null
}

export interface LeaveOutEdits {
  added: string[]
  removed: string[]
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

export interface Count {
  files: number
  bytes: number
}

export type Change =
  | 'send'
  | 'receive'
  | 'staysHere'
  | 'staysThere'
  | { replace: { winnerHere: boolean } }

export interface Group {
  rel: string
  mode: string
  send: Count
  receive: Count
  replacedHere: Count
  replacedThere: Count
  staysHere: Count
  staysThere: Count
  same: Count
  filesHere: number
  filesThere: number
  samples: { path: string; change: Change; size: number }[]
}

export interface Preview {
  groups: Group[]
  guard: string[]
  windowsNames: Problem[]
  otherUnknown: boolean
}

export interface PlanView {
  plan: Plan
  preview: Preview
  edits: EditView[]
  leaveOut: LeaveOutEdits
}

export interface Report {
  added: number
  updated: number
  removed: number
}

export interface Conflict {
  copy: string
  original: string
  copySize: number
  copyMtime: number
  originalSize: number | null
  originalMtime: number | null
}

export interface Kept {
  folder: string
  rel: string
  path: string
  versionTime: string
  modTime: string
  size: number
}

export type Restored = 'inPlace' | { beside: string }

export interface FolderCheck {
  id: string
  rel: string
  folderType: string
  state: string
  needFiles: number
  needBytes: number
  otherCompletion: number | null
  localChanges: number
  errors: string[]
  samples: string[]
  ok: boolean
}

export const api = {
  state: () => invoke<View>('state'),
  refresh: () => invoke<View>('refresh'),
  children: (path: string) => invoke<NodeView[]>('children', { path }),
  setRule: (path: string, dir: boolean, mode: string | null) => invoke<View>('set_rule', { path, dir, mode }),
  setLeaveOut: (list: string[]) => invoke<View>('set_leave_out', { list }),
  setPause: (until: number | null) => invoke<View>('set_pause', { until }),
  discard: () => invoke<View>('discard'),
  plan: () => invoke<PlanView>('plan'),
  apply: (guardConfirmed: boolean) => invoke<Report>('apply', { guardConfirmed }),
  init: (root: string) => invoke<View>('init', { root }),
  setAllow: (what: 'metered' | 'relay', on: boolean) => invoke<View>('set_allow', { what, on }),
  resume: (folder: string) => invoke<View>('resume', { folder }),
  createRules: (otherOs: Os) => invoke<View>('create_rules', { otherOs }),
  conflicts: () => invoke<Conflict[]>('conflicts'),
  keep: (copy: string, keepCopy: boolean) => invoke<Conflict[]>('keep', { copy, keepCopy }),
  versions: () => invoke<Kept[]>('versions'),
  restore: (folder: string, path: string, versionTime: string) => invoke<Restored>('restore', { folder, path, versionTime }),
  verify: (rescan: boolean) => invoke<FolderCheck[]>('verify', { rescan }),
}
