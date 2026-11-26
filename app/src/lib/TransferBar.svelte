<script lang="ts">
  import NumberFlow from '@number-flow/svelte'
  import type { Flow, Snapshot } from './api'
  import { bytes, duration, sizeParts } from './format'

  let { snap, there, onopen }: { snap: Snapshot; there: string; onopen: () => void } = $props()

  /** How many speed samples the little graph keeps (Setu sends one every few seconds). */
  const SAMPLES = 40

  /**
   * Add one speed reading to a lane's history and return the new history.
   * The graph draws whatever this returns, oldest first.
   */
  function pushSample(history: number[], speed: number): number[] {
    // Light smoothing: Syncthing copies in blocks, so raw speed jumps; 0.4 still shows a real slowdown within a few readings.
    const prev = history.at(-1)
    const v = prev === undefined ? speed : 0.4 * speed + 0.6 * prev
    return [...history, v].slice(-SAMPLES)
  }

  let downHist = $state<number[]>([])
  let upHist = $state<number[]>([])
  let lastAt = 0

  $effect(() => {
    if (snap.at === lastAt) return
    lastAt = snap.at
    downHist = snap.down.active ? pushSample(downHist, snap.down.speed) : []
    upHist = snap.up.active ? pushSample(upHist, snap.up.speed) : []
  })

  /** SVG path for a speed graph 100 wide and 20 high. */
  function spark(h: number[]): string {
    if (h.length < 2) return ''
    const max = Math.max(...h, 1)
    const step = 100 / (SAMPLES - 1)
    const x0 = 100 - (h.length - 1) * step
    return h.map((v, i) => `${i ? 'L' : 'M'}${(x0 + i * step).toFixed(1)},${(19 - (v / max) * 17).toFixed(1)}`).join(' ')
  }

  const lanes = $derived(
    [
      { key: 'down', label: 'To this computer', arrow: '↓', tone: 'in', flow: snap.down, hist: downHist },
      { key: 'up', label: `To ${there}`, arrow: '↑', tone: 'out', flow: snap.up, hist: upHist },
    ].filter((l) => l.flow.active),
  )

  function left(f: Flow): string {
    if (f.etaSecs !== null) return `${duration(f.etaSecs)} left`
    return f.speed ? 'working out time left' : 'starting'
  }
</script>

<button class="tbar" onclick={onopen} title="Open copy progress: speed, time so far, time left, per folder">
  {#each lanes as l (l.key)}
    {@const pct = Math.min(100, Math.max(0, l.flow.percent))}
    {@const line = spark(l.hist)}
    {@const sp = sizeParts(l.flow.speed)}
    <div class="lane {l.tone}" role="progressbar" aria-label={l.label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.floor(pct)} aria-valuetext={`${Math.floor(pct)}% of ${bytes(l.flow.totalBytes)}, ${left(l.flow)}`}>
      <span class="who"><span class="arrow" aria-hidden="true">{l.arrow}</span>{l.label}</span>
      <span class="track" aria-hidden="true">
        <span class="fill" style="transform:translateX({pct - 100}%)"><span class="head"></span></span>
        <span class="ticks"></span>
      </span>
      <span class="pct"><NumberFlow value={Math.floor(pct)} /><small>%</small></span>
      <svg class="spark" viewBox="0 0 100 20" preserveAspectRatio="none" aria-hidden="true">
        {#if line}<path class="area" d="{line} L100,20 L{100 - (l.hist.length - 1) * (100 / (SAMPLES - 1))},20 Z" /><path class="ln" d={line} />{/if}
      </svg>
      <span class="num speed"><NumberFlow value={sp.value} format={{ minimumFractionDigits: sp.decimals, maximumFractionDigits: sp.decimals }} suffix={` ${sp.unit}/s`} /></span>
      <span class="num meta">{bytes(l.flow.doneBytes)} / {bytes(l.flow.totalBytes)} · {left(l.flow)}</span>
    </div>
  {/each}
</button>

<style>
  .tbar {
    display: grid;
    grid-template-columns: minmax(96px, auto) minmax(80px, 1fr) 44px 72px 76px minmax(0, auto);
    column-gap: 10px;
    row-gap: 4px;
    flex: 1;
    min-width: 0;
    padding: 6px 12px;
    border-radius: 12px;
    border: 1px solid color-mix(in srgb, var(--accent) 22%, var(--line));
    background:
      linear-gradient(90deg, color-mix(in srgb, var(--accent) 7%, transparent), transparent 60%),
      var(--surface);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, #fff 4%, transparent), 0 0 18px -8px var(--accent);
    text-align: left;
    position: relative;
    transition: transform 120ms ease-out, border-color 150ms ease-out;
  }
  .tbar:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--line));
  }
  .tbar:active {
    transform: scale(0.99);
  }
  /* Border beam: a short light that runs round the edge while copying. */
  .tbar::before {
    content: '';
    position: absolute;
    inset: -1px;
    border-radius: inherit;
    padding: 1px;
    background: conic-gradient(from var(--beam), transparent 0 80%, var(--in) 88%, var(--out) 94%, transparent 100%);
    mask: linear-gradient(#000 0 0) content-box exclude, linear-gradient(#000 0 0);
    animation: beam 4s linear infinite;
    pointer-events: none;
  }
  @property --beam {
    syntax: '<angle>';
    initial-value: 0deg;
    inherits: false;
  }
  @keyframes beam {
    to {
      --beam: 360deg;
    }
  }
  .lane {
    --c: var(--accent);
    /* Subgrid: both lanes share the bar's columns, so the two tracks line up. */
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: subgrid;
    align-items: center;
    min-width: 0;
    font-size: 12.5px;
  }
  .lane.in {
    --c: var(--in);
  }
  .lane.out {
    --c: var(--out);
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .arrow {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 5px;
    color: var(--c);
    background: color-mix(in srgb, var(--c) 16%, transparent);
    font-size: 11px;
  }
  .track {
    position: relative;
    height: 8px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--c) 12%, var(--surface-2));
    overflow: hidden;
  }
  .fill {
    position: relative;
    display: block;
    height: 100%;
    border-radius: inherit;
    background:
      repeating-linear-gradient(115deg, transparent 0 6px, color-mix(in srgb, #fff 18%, transparent) 6px 9px),
      linear-gradient(90deg, color-mix(in srgb, var(--c) 55%, transparent), var(--c));
    background-size: 24px 100%, 100% 100%;
    box-shadow: 0 0 10px color-mix(in srgb, var(--c) 60%, transparent);
    transition: transform 600ms ease-out;
    animation: flow 900ms linear infinite;
  }
  .head {
    position: absolute;
    right: -1px;
    top: -2px;
    bottom: -2px;
    width: 3px;
    border-radius: 2px;
    background: #fff;
    box-shadow: 0 0 8px 2px var(--c);
  }
  .ticks {
    position: absolute;
    inset: 0;
    background: repeating-linear-gradient(90deg, transparent 0 calc(10% - 1px), color-mix(in srgb, var(--bg, #000) 35%, transparent) calc(10% - 1px) 10%);
    pointer-events: none;
  }
  .pct {
    color: var(--c);
    font: 700 15px/1 ui-monospace, 'Cascadia Code', 'JetBrains Mono', monospace;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .pct small {
    font-size: 10px;
    opacity: 0.7;
  }
  .spark {
    width: 72px;
    height: 20px;
  }
  .spark .ln {
    fill: none;
    stroke: var(--c);
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
  }
  .spark .area {
    fill: color-mix(in srgb, var(--c) 18%, transparent);
  }
  .num {
    font-family: ui-monospace, 'Cascadia Code', 'JetBrains Mono', monospace;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .speed {
    color: var(--c);
    font-weight: 600;
    text-align: right;
  }
  .meta {
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  @keyframes flow {
    to {
      background-position: 24px 0, 0 0;
    }
  }
  @media (max-width: 760px) {
    .tbar {
      grid-template-columns: minmax(80px, auto) minmax(60px, 1fr) 40px 70px;
    }
    .spark,
    .meta {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .fill,
    .tbar::before {
      animation: none;
      transition: none;
    }
    .tbar::before {
      display: none;
    }
  }
</style>
