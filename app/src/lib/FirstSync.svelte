<script lang="ts">
  import type { DeviceView, NodeView } from './api'
  import DirPicker from './DirPicker.svelte'
  import { bytes, dirOf, sentence, type Dir } from './format'

  let {
    folders,
    me,
    other,
    remoteKnown,
    onpick,
    onclose,
    onreview,
  }: {
    folders: NodeView[]
    me: DeviceView
    other: DeviceView
    remoteKnown: boolean
    onpick: (n: NodeView, d: Dir) => Promise<void>
    onclose: () => void
    onreview: () => void
  } = $props()

  // Biggest first: they matter most and are the slowest to sync.
  const order = $derived(
    [...folders].sort((a, b) => Math.max(b.here?.size ?? 0, b.there?.size ?? 0) - Math.max(a.here?.size ?? 0, a.there?.size ?? 0)),
  )
  let i = $state(0)
  const n = $derived(order[i])
  const d = $derived(n ? dirOf(n.effective, me.id) : 'off')
  const where = (x: NodeView) =>
    x.here && x.there ? 'On both computers' : x.here ? `Only on ${me.name}` : `Only on ${other.name}`
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-labelledby="fs-title">
  <header>
    <p class="muted small">First sync · folder {i + 1} of {order.length}</p>
    <h2 id="fs-title">{n?.name ?? 'No folders'}</h2>
    <div class="bar"><span style="width:{order.length ? ((i + 1) / order.length) * 100 : 0}%"></span></div>
  </header>
  {#if n}
    <div class="body">
      <p class="where">{where(n)}</p>
      <dl>
        <dt>{me.name}</dt>
        <dd>{n.here ? `${bytes(n.here.size)} · ${n.here.files.toLocaleString()} files` : '—'}</dd>
        <dt>{other.name}</dt>
        <dd>{n.there ? `${bytes(n.there.size)} · ${n.there.files.toLocaleString()} files` : remoteKnown ? '—' : 'list not arrived'}</dd>
      </dl>
      <DirPicker value={d} meName={me.name} otherName={other.name} onpick={(x) => onpick(n, x)} />
      <p class="muted small say">{sentence(d, me.name, other.name)}</p>
      <p class="muted small">Leave it Off to decide later. You can open a folder in the tree to choose for what is inside.</p>
    </div>
  {/if}
  <footer>
    <button class="btn" onclick={onclose}>Close</button>
    <div class="grow"></div>
    <button class="btn" disabled={i === 0} onclick={() => i--}>Back</button>
    {#if i < order.length - 1}
      <button class="btn primary" onclick={() => i++}>Next</button>
    {:else}
      <button class="btn primary" onclick={onreview}>Review and apply</button>
    {/if}
  </footer>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgb(10 12 20 / 45%);
  }
  .sheet {
    position: fixed;
    top: 50%;
    left: 50%;
    width: min(520px, calc(100vw - 32px));
    transform: translate(-50%, -50%);
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 16px;
    box-shadow: var(--shadow);
  }
  header {
    padding: 20px 24px 6px;
  }
  header p {
    margin: 0;
  }
  h2 {
    margin: 4px 0 12px;
    font-size: 20px;
    overflow-wrap: anywhere;
  }
  .bar {
    height: 4px;
    border-radius: 4px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 200ms ease;
  }
  .body {
    padding: 12px 24px;
  }
  .where {
    margin: 0 0 8px;
    font-weight: 600;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 14px;
    margin: 0 0 16px;
    font-size: 13px;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
  .say {
    margin: 12px 0 4px;
  }
  .small {
    font-size: 12.5px;
  }
  footer {
    display: flex;
    gap: 8px;
    padding: 14px 24px 18px;
    border-top: 1px solid var(--line);
  }
  .grow {
    flex: 1;
  }
</style>
