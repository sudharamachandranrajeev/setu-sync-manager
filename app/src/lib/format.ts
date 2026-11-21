import type { DeviceView } from './api'

export type Dir = 'off' | 'both' | 'out' | 'in'

/** A mode string from the rules, seen from this computer: out = this computer sends. */
export function dirOf(mode: string | null, meId: string | undefined): Dir {
  if (!mode || mode === 'off') return 'off'
  if (mode === 'both') return 'both'
  return mode === `from:${meId}` ? 'out' : 'in'
}

export function modeOf(dir: Dir, me: DeviceView, other: DeviceView): string {
  switch (dir) {
    case 'off':
      return 'off'
    case 'both':
      return 'both'
    case 'out':
      return `from:${me.id}`
    case 'in':
      return `from:${other.id}`
  }
}

/**
 * What to call the other computer in everyday words: "Windows", "Linux" or "Mac" when the two
 * computers run different systems, otherwise its device name.
 */
export function called(other: DeviceView, me: DeviceView): string {
  if (other.os === me.os) return other.name
  return { windows: 'Windows', linux: 'Linux', macos: 'Mac' }[other.os]
}

export const glyph: Record<Dir, string> = { off: '–', both: '⇄', out: '→', in: '←' }

/** Short label for chips in the list. */
export function shortLabel(d: Dir, there: string): string {
  return { off: 'Not synced', both: 'Both ways', out: `To ${there}`, in: `From ${there}` }[d]
}

/** Label for the choice buttons. */
export function choiceLabel(d: Dir, there: string): string {
  return { off: "Don't sync", both: 'Both ways', out: `Copy to ${there}`, in: `Copy from ${there}` }[d]
}

/** What a choice does, in full. */
export function sentence(d: Dir, there: string): string {
  switch (d) {
    case 'off':
      return `Not synced. Each computer keeps its own copy, and nothing is copied either way.`
    case 'both':
      return `Both computers keep the same files. A change or delete on either one is copied to the other.`
    case 'out':
      return `This computer's files are copied to ${there}. Edits and deletes made here reach ${there}. Edits made on ${there} stay there and are not copied back.`
    case 'in':
      return `${there}'s files are copied to this computer. Edits and deletes made on ${there} reach here. Edits you make here stay here and are not copied back.`
  }
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`
  const u = ['KB', 'MB', 'GB', 'TB']
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024
    i++
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${u[i]}`
}

export function ago(secs: number | null): string {
  if (secs === null) return 'never'
  if (secs < 90) return 'just now'
  if (secs < 3600) return `${Math.round(secs / 60)} min ago`
  if (secs < 86400 * 2) return `${Math.round(secs / 3600)} h ago`
  return `${Math.round(secs / 86400)} days ago`
}

/** Syncthing's folder states in everyday words. */
export function stateWord(s: string): string {
  return (
    {
      idle: 'up to date',
      syncing: 'copying',
      'sync-preparing': 'getting ready to copy',
      'sync-waiting': 'waiting its turn',
      scanning: 'looking for changes',
      'scan-waiting': 'waiting to look for changes',
      cleaning: 'tidying up',
      paused: 'paused',
      missing: 'not set up on this computer yet',
      error: 'stopped by an error',
    } as Record<string, string>
  )[s] ?? s
}
