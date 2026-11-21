<script lang="ts">
  import type { NodeView } from './api'
  import { bytes, dirOf, glyph, shortLabel } from './format'

  export interface Row {
    node: NodeView
    depth: number
    open: boolean
    loading: boolean
  }

  let {
    rows,
    selected,
    meId,
    there,
    remoteKnown,
    onselect,
    ontoggle,
  }: {
    rows: Row[]
    selected: string | null
    meId: string
    /** Everyday name of the other computer. */
    there: string
    remoteKnown: boolean
    onselect: (n: NodeView) => void
    ontoggle: (n: NodeView) => void
  } = $props()

  const ROW = 34
  let scrollTop = $state(0)
  let height = $state(600)
  let start = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 8))
  let end = $derived(Math.min(rows.length, Math.ceil((scrollTop + height) / ROW) + 8))

  function key(e: KeyboardEvent) {
    const i = rows.findIndex((r) => r.node.path === selected)
    if (e.key === 'ArrowDown' && i < rows.length - 1) onselect(rows[i + 1].node)
    else if (e.key === 'ArrowUp' && i > 0) onselect(rows[i - 1].node)
    else if ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && i >= 0) {
      const r = rows[i]
      if (r.node.dir && r.open === (e.key === 'ArrowLeft')) ontoggle(r.node)
    } else return
    e.preventDefault()
  }
</script>

<div class="head">
  <span>Name</span>
  <span class="num" title="Size on this computer">This PC</span>
  <span class="num" title="Size on {there}">{there}</span>
  <span class="dirh">Sync</span>
</div>
<div
  class="scroll"
  role="tree"
  tabindex="0"
  aria-label="Files and folders"
  bind:clientHeight={height}
  onscroll={(e) => (scrollTop = (e.currentTarget as HTMLElement).scrollTop)}
  onkeydown={key}
>
  <div style="height:{rows.length * ROW}px; position:relative">
    {#each rows.slice(start, end) as r, i (r.node.path)}
      {@const n = r.node}
      {@const d = dirOf(n.effective, meId)}
      <div
        class="row"
        role="treeitem"
        aria-selected={selected === n.path}
        aria-expanded={n.dir ? r.open : undefined}
        tabindex="-1"
        class:sel={selected === n.path}
        style="top:{(start + i) * ROW}px"
        onclick={() => onselect(n)}
        ondblclick={() => n.dir && ontoggle(n)}
        onkeydown={() => {}}
      >
        <span class="name" style="padding-left:{8 + r.depth * 18}px">
          {#if n.dir}
            <button
              tabindex="-1"
              class="chev"
              class:open={r.open}
              aria-label={r.open ? 'Collapse' : 'Expand'}
              onclick={(e) => {
                e.stopPropagation()
                ontoggle(n)
              }}
            >
              {#if r.loading}<span class="spin"></span>{:else}<svg viewBox="0 0 16 16" width="12" height="12"><path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>{/if}
            </button>
            <svg class="ico folder" viewBox="0 0 20 20" width="17" height="17"><path d="M2.5 5.5a1.5 1.5 0 0 1 1.5-1.5h3.6l1.7 1.8h6.7a1.5 1.5 0 0 1 1.5 1.5v7.7a1.5 1.5 0 0 1-1.5 1.5H4a1.5 1.5 0 0 1-1.5-1.5z" fill="currentColor" /></svg>
          {:else}
            <span class="chev"></span>
            <svg class="ico file" viewBox="0 0 20 20" width="16" height="16"><path d="M5 2.5h6.5L15.5 6.5v11H5z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" /><path d="M11.5 2.5v4h4" fill="none" stroke="currentColor" stroke-width="1.4" /></svg>
          {/if}
          <span class="label" title={n.path}>{n.name}</span>
          {#if !n.here}<span class="only">only on {there}</span>{/if}
          {#if remoteKnown && !n.there && n.here}<span class="only">only here</span>{/if}
        </span>
        <span class="num" class:none={!n.here}>{n.here ? bytes(n.here.size) : '—'}</span>
        <span class="num" class:none={!n.there}>{n.there ? bytes(n.there.size) : '—'}</span>
        <span class="dir">
          <span class="chip {d}" class:set={n.explicit !== null} title={n.explicit ? 'Chosen for this item' : 'Follows the folder it is in'}>
            <span class="g">{glyph[d]}</span>{shortLabel(d, there)}
          </span>
        </span>
      </div>
    {/each}
  </div>
</div>

<style>
  .head,
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 84px 84px 136px;
    column-gap: 6px;
    align-items: center;
  }
  @media (max-width: 900px) {
    .head,
    .row {
      grid-template-columns: minmax(0, 1fr) 70px 70px 124px;
    }
  }
  .head {
    height: 34px;
    padding: 0 12px 0 0;
    border-bottom: 1px solid var(--line);
    color: var(--faint);
    font-size: 11.5px;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .head > span:first-child {
    padding-left: 40px;
  }
  .scroll {
    flex: 1;
    overflow: auto;
    outline: none;
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 34px;
    padding-right: 12px;
    border-radius: 7px;
    cursor: default;
    user-select: none;
  }
  .row:hover {
    background: var(--surface-2);
  }
  .row.sel {
    background: color-mix(in srgb, var(--accent) 13%, transparent);
  }
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .chev {
    width: 18px;
    height: 18px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 5px;
    color: var(--faint);
    transition: transform 140ms ease;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  button.chev:hover {
    background: var(--line);
    color: var(--text);
  }
  .ico {
    flex: none;
  }
  .folder {
    color: color-mix(in srgb, var(--accent) 70%, var(--faint));
  }
  .file {
    color: var(--faint);
  }
  .label {
    flex: 0 1 auto;
    min-width: 2.5em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .only {
    flex: 0 100 auto;
    /* Wide enough for "only here"; longer ones shorten with an ellipsis. */
    min-width: 72px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--muted);
    border: 1px solid var(--line);
  }
  .num {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--muted);
    font-size: 12.5px;
  }
  .num.none {
    color: var(--faint);
  }
  .dir,
  .dirh {
    display: flex;
    justify-content: flex-end;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 9px;
    border-radius: 99px;
    max-width: 100%;
    overflow: hidden;
    white-space: nowrap;
    font-size: 12px;
    font-weight: 600;
    border: 1px dashed currentColor;
    opacity: 0.75;
  }
  .chip.set {
    border-style: solid;
    opacity: 1;
    background: color-mix(in srgb, currentColor 12%, transparent);
  }
  .chip .g {
    font-size: 13px;
  }
  .off {
    color: var(--off);
  }
  .both {
    color: var(--both);
  }
  .out {
    color: var(--out);
  }
  .in {
    color: var(--in);
  }
  .spin {
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 2px solid var(--line);
    border-top-color: var(--accent);
    animation: spin 700ms linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
