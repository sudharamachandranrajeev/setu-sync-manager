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
  if (!other.os || other.os === me.os) return other.name
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

/** How a computer is doing with the current sync choices, in a few words. */
export function deviceState(d: DeviceView, revision: number | null): { text: string; tone: 'ok' | 'busy' | 'wait' } {
  if (d.held) return { text: 'needs you', tone: 'wait' }
  if (!d.inRules) return { text: 'not syncing yet', tone: 'wait' }
  if (revision !== null && d.appliedRevision !== null && d.appliedRevision >= revision) return { text: 'up to date', tone: 'ok' }
  return { text: 'setting up', tone: 'busy' }
}

/** A byte count as a number and a unit, rounded the way `bytes` shows it: `{ value: 2.1, unit: 'GB' }`. */
export function sizeParts(n: number): { value: number; unit: string; decimals: number } {
  if (n < 1024) return { value: n, unit: 'B', decimals: 0 }
  const u = ['KB', 'MB', 'GB', 'TB']
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024
    i++
  }
  const decimals = v < 10 ? 1 : 0
  return { value: Number(v.toFixed(decimals)), unit: u[i], decimals }
}

export function bytes(n: number): string {
  const { value, unit, decimals } = sizeParts(n)
  return `${value.toFixed(decimals)} ${unit}`
}

/** `42 s`, `3 min 12 s`, `2 h 05 min`. */
export function duration(secs: number): string {
  secs = Math.max(0, Math.round(secs))
  if (secs < 60) return `${secs} s`
  if (secs < 3600) return `${Math.floor(secs / 60)} min ${String(secs % 60).padStart(2, '0')} s`
  return `${Math.floor(secs / 3600)} h ${String(Math.floor((secs % 3600) / 60)).padStart(2, '0')} min`
}

/** Bytes a second, e.g. `12 MB/s`. */
export function rate(n: number): string {
  return `${bytes(n)}/s`
}

/** A clock time for a Unix time, with the day when it is not today: `14:32`, `tomorrow 07:00`. */
export function clock(unix: number): string {
  const d = new Date(unix * 1000)
  const t = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  const today = new Date()
  if (d.toDateString() === today.toDateString()) return t
  const tomorrow = new Date(today)
  tomorrow.setDate(today.getDate() + 1)
  if (d.toDateString() === tomorrow.toDateString()) return `tomorrow ${t}`
  return `${d.toLocaleDateString([], { day: 'numeric', month: 'short' })} ${t}`
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
