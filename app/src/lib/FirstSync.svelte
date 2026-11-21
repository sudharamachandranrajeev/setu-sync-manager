<script lang="ts">
  import { untrack } from 'svelte'
  import type { DeviceView, NodeView } from './api'
  import DirPicker from './DirPicker.svelte'
  import { bytes, dirOf, sentence, type Dir } from './format'

  let {
    folders,
    me,
    there,
    remoteKnown,
    rootDir,
    onroot,
    onpick,
    onclose,
    onreview,
  }: {
    /** Top-level folders. */
    folders: NodeView[]
    me: DeviceView
    /** Everyday name of the other computer. */
    there: string
    remoteKnown: boolean
    rootDir: Dir
    onroot: (d: Dir) => Promise<void>
    onpick: (n: NodeView, d: Dir | null) => Promise<void>
    onclose: () => void
    onreview: () => void
  } = $props()

  type Step = 'what' | 'leave' | 'each'
  let step = $state<Step>('what')
  // The current choice is only the starting point.
  let choice = $state<Dir | null>(untrack(() => (rootDir === 'off' ? null : rootDir)))
  let working = $state(false)
  const BIG = 1024 ** 3

  const size = (n: NodeView) => Math.max(n.here?.size ?? 0, n.there?.size ?? 0)
  // Biggest first: they matter most and are the slowest to copy.
  const order = $derived([...folders].sort((a, b) => size(b) - size(a)))
  // Folders already switched off stay unticked.
  let include = $state<Record<string, boolean>>({})
  $effect.pre(() => {
    for (const f of folders) if (!(f.path in include)) include[f.path] = dirOf(f.effective, me.id) !== 'off' || rootDir === 'off'
  })

  /** Size of what gets copied for a folder, from the side that sends. */
  const sent = (n: NodeView) => (choice === 'in' ? n.there : choice === 'out' ? n.here : n.here && n.there ? (n.here.size >= n.there.size ? n.here : n.there) : (n.here ?? n.there))
  const total = $derived(order.filter((f) => include[f.path]).reduce((s, f) => s + (sent(f)?.size ?? 0), 0))
  const leftOut = $derived(order.filter((f) => !include[f.path]).length)
  const picked = $derived(order.length - leftOut)

  function setAll(on: boolean) {
    for (const f of order) include[f.path] = on
  }

  const choices: { d: Dir; title: string; body: string }[] = $derived([
    { d: 'in', title: `Copy everything from ${there} to this computer`, body: `${there} is the main copy. Edits you make here are not sent back.` },
    { d: 'out', title: `Copy everything from this computer to ${there}`, body: `This computer is the main copy. Edits made on ${there} are not sent back.` },
    { d: 'both', title: 'Keep everything the same on both', body: 'Work on either computer; changes go both ways.' },
  ])

  async function finish() {
    if (!choice) return
    working = true
    try {
      await onroot(choice)
      for (const f of order) await onpick(f, include[f.path] ? null : 'off')
      onreview()
    } finally {
      working = false
    }
  }

  // Folder-by-folder mode.
  let i = $state(0)
  const n = $derived(order[i])
  const d = $derived(n ? dirOf(n.effective, me.id) : 'off')
  const where = (x: NodeView) => (x.here && x.there ? 'On both computers' : x.here ? 'Only on this computer' : `Only on ${there}`)
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-labelledby="fs-title">
  {#if step === 'what'}
    <header>
      <p class="muted small">Quick setup · step 1 of 2</p>
      <h2 id="fs-title">What should sync?</h2>
    </header>
    <div class="body">
      <div class="choices" role="radiogroup" aria-label="What should sync">
        {#each choices as c (c.d)}
          <button role="radio" aria-checked={choice === c.d} class="choice {c.d}" class:on={choice === c.d} onclick={() => (choice = c.d)}>
            <b>{c.title}</b>
            <span class="muted small">{c.body}</span>
          </button>
        {/each}
        <button class="choice plain" onclick={() => ((step = 'each'), (i = 0))}>
          <b>Choose folder by folder</b>
          <span class="muted small">Go through the top folders one at a time, biggest first.</span>
        </button>
      </div>
      <p class="muted small">Nothing is copied until you review and press Start syncing.</p>
    </div>
    <footer>
      <button class="btn" onclick={onclose}>Close</button>
      <div class="grow"></div>
      <button class="btn primary" disabled={!choice} onclick={() => (step = 'leave')}>Next</button>
    </footer>
  {:else if step === 'leave'}
    <header>
      <p class="muted small">Quick setup · step 2 of 2</p>
      <h2 id="fs-title">Leave anything out?</h2>
    </header>
    <div class="body">
      <p class="small">Tick the folders to copy and untick the ones to leave out, such as big projects. Biggest are at the top.</p>
      {#if !remoteKnown}
        <p class="note small">{there}'s file list has not arrived yet, so sizes show this computer only. It arrives by itself while Setu runs on {there}.</p>
      {/if}
      <div class="allrow">
        <label>
          <input
            type="checkbox"
            checked={picked === order.length && order.length > 0}
            indeterminate={picked > 0 && picked < order.length}
            onchange={(e) => setAll((e.currentTarget as HTMLInputElement).checked)}
          />
          <b>{picked} of {order.length} folders selected</b>
        </label>
        <div class="grow"></div>
        <button class="link-btn" disabled={picked === order.length} onclick={() => setAll(true)}>Select all</button>
        <button class="link-btn" disabled={picked === 0} onclick={() => setAll(false)}>Select none</button>
      </div>
      <ul class="pick">
        {#each order as f (f.path)}
          {@const s = sent(f)}
          <li>
            <label>
              <input type="checkbox" bind:checked={include[f.path]} />
              <span class="fname">{f.name}</span>
              {#if (s?.size ?? 0) >= BIG}<span class="big">large</span>{/if}
              <span class="fsize muted small">{s ? `${bytes(s.size)} · ${s.files.toLocaleString()} files` : choice === 'in' ? `not on ${there}` : choice === 'out' ? 'not on this computer' : 'empty'}</span>
            </label>
          </li>
        {/each}
      </ul>
      {#if picked === 0}
        <p class="note small">No folders selected. Only loose files at the top level will sync. Tick the folders you want copied.</p>
      {/if}
      <p class="sum small">
        <b>About {bytes(total)}</b> to look at{leftOut ? `, ${leftOut} folder${leftOut === 1 ? '' : 's'} left out` : ''}. Files that are already the same are not copied again.
      </p>
      <p class="muted small">
        Always left out automatically: secrets (.env, keys) and rebuildable clutter (.git, node_modules, build, target, dist). To leave out
        something deeper, open a folder in the main list later and choose Don't sync.
      </p>
    </div>
    <footer>
      <button class="btn" onclick={() => (step = 'what')}>Back</button>
      <div class="grow"></div>
      <button class="btn primary" disabled={working} onclick={finish}>{working ? 'Setting up…' : 'Review'}</button>
    </footer>
  {:else}
    <header>
      <p class="muted small">Folder {i + 1} of {order.length}</p>
      <h2 id="fs-title">{n?.name ?? 'No folders'}</h2>
      <div class="bar"><span style="width:{order.length ? ((i + 1) / order.length) * 100 : 0}%"></span></div>
    </header>
    {#if n}
      <div class="body">
        <p class="where">{where(n)}</p>
        <dl>
          <dt>This computer</dt>
          <dd>{n.here ? `${bytes(n.here.size)} · ${n.here.files.toLocaleString()} files` : '—'}</dd>
          <dt>{there}</dt>
          <dd>{n.there ? `${bytes(n.there.size)} · ${n.there.files.toLocaleString()} files` : remoteKnown ? '—' : 'list not arrived yet'}</dd>
        </dl>
        <DirPicker value={d} {there} onpick={(x) => onpick(n, x)} />
        <p class="muted small say">{sentence(d, there)}</p>
        <p class="muted small">Not sure? Leave it as Don't sync and decide later.</p>
      </div>
    {/if}
    <footer>
      <button class="btn" onclick={() => (i === 0 ? (step = 'what') : i--)}>Back</button>
      <div class="grow"></div>
      {#if i < order.length - 1}
        <button class="btn primary" onclick={() => i++}>Next</button>
      {:else}
        <button class="btn primary" onclick={onreview}>Review</button>
      {/if}
    </footer>
  {/if}
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
    width: min(560px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    display: flex;
    flex-direction: column;
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
    padding: 8px 24px 12px;
    overflow: auto;
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 12px;
  }
  .choice {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 14px;
    text-align: left;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--surface);
  }
  .choice:hover {
    border-color: var(--faint);
  }
  .choice.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .choice.plain {
    background: var(--surface-2);
  }
  .allrow {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 8px;
    padding: 0 12px;
  }
  .allrow label {
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
  }
  .link-btn {
    color: var(--accent);
    font-weight: 600;
    font-size: 12.5px;
    padding: 0;
  }
  .link-btn:hover:not(:disabled) {
    text-decoration: underline;
  }
  .link-btn:disabled {
    color: var(--faint);
  }
  .pick {
    list-style: none;
    margin: 8px 0;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: 10px;
    max-height: 300px;
    overflow: auto;
  }
  .pick li + li {
    border-top: 1px solid var(--line);
  }
  .pick label {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    cursor: pointer;
  }
  .fname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .fsize {
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .big {
    font-size: 11px;
    font-weight: 650;
    padding: 1px 7px;
    border-radius: 99px;
    color: var(--warn);
    background: var(--warn-bg);
  }
  .sum {
    margin: 8px 0;
  }
  .note {
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--warn-bg);
    color: var(--warn);
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
