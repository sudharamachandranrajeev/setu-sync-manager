import { invoke } from '@tauri-apps/api/core'

export type Os = 'linux' | 'windows' | 'macos'

export interface Held {
  revision: number
  reasons: string[]
}

export interface DeviceView {
  id: string
  short: string
  name: string
  os: Os | null
  connected: boolean
  address: string | null
  appliedRevision: number | null
  appliedAt: number | null
  /** Named in the sync choices. */
  inRules: boolean
  /** A backup copy: only receives, never sends changes back. */
  receiveOnly: boolean
  /** New choices wait for the person on that computer, and why. */
  held: Held | null
  problems: string[]
  /** Last report to the Brahma account (Unix seconds). */
  seenAt: number | null
}

export interface AccountView {
  server: string
  email: string
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
  stage: 'setup' | 'signin' | 'join' | 'ready'
  account: AccountView | null
  /** Computers besides this one and the main other one (or all, before joining). */
  devices: DeviceView[]
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

export interface AccuracyRow {
  top: string
  files: number
  bytes: number
  checkedAt: number
  /** [id, name, 'same' | 'differs' | 'syncing' | 'unchecked'] per other computer. */
  others: [string, string, 'same' | 'differs' | 'syncing' | 'unchecked'][]
}

export interface Mismatch {
  path: string
  what: string
}

export interface Removed {
  at: number
  kind: string
  removed: string
  kept: string
  size: number
  sha256: string
  undone: boolean
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
  signIn: (server: string, email: string, password: string) => invoke<View>('sign_in', { server, email, password }),
  recovery: (key: string) => invoke<View>('recovery', { key }),
  signOut: () => invoke<View>('sign_out'),
  join: (receiveOnly: boolean) => invoke<View>('join', { receiveOnly }),
  checkNow: () => invoke<void>('check_now'),
  accuracy: () => invoke<AccuracyRow[]>('accuracy'),
  differences: (device: string, top: string) => invoke<Mismatch[] | null>('differences', { device, top }),
  tidyNow: () => invoke<Removed[]>('tidy_now'),
  tidyLog: () => invoke<Removed[]>('tidy_log'),
  undoTidy: (path: string) => invoke<Removed[]>('undo_tidy', { path }),
  conflicts: () => invoke<Conflict[]>('conflicts'),
  keep: (copy: string, keepCopy: boolean) => invoke<Conflict[]>('keep', { copy, keepCopy }),
  versions: () => invoke<Kept[]>('versions'),
  restore: (folder: string, path: string, versionTime: string) => invoke<Restored>('restore', { folder, path, versionTime }),
  verify: (rescan: boolean) => invoke<FolderCheck[]>('verify', { rescan }),
}
