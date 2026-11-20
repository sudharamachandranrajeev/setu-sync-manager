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

export const glyph: Record<Dir, string> = { off: '–', both: '⇄', out: '→', in: '←' }

export function shortLabel(d: Dir): string {
  return { off: 'Off', both: 'Both ways', out: 'Send', in: 'Receive' }[d]
}

export function sentence(d: Dir, me: string, other: string): string {
  switch (d) {
    case 'off':
      return `Not synced. ${me} and ${other} each keep their own copy.`
    case 'both':
      return `Changes made on either computer reach the other.`
    case 'out':
      return `${me} sends and ${other} receives. Changes made on ${other} are not sent back.`
    case 'in':
      return `${other} sends and ${me} receives. Changes made here are not sent back.`
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
