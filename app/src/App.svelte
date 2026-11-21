<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type NodeView, type Os, type PlanView, type Report, type View } from './lib/api'
  import DirPicker from './lib/DirPicker.svelte'
  import FirstSync from './lib/FirstSync.svelte'
  import PlanSheet from './lib/PlanSheet.svelte'
  import Tree, { type Row } from './lib/Tree.svelte'
  import { ago, bytes, dirOf, modeOf, sentence, shortLabel, type Dir } from './lib/format'

  let view = $state<View | null>(null)
  let rows = $state<Row[]>([])
  let selected = $state<string | null>(null)
  let plan = $state<PlanView | null>(null)
  let busy = $state(false)
  let error = $state<string | null>(null)
  let toast = $state<string | null>(null)
  let rootInput = $state('')
  let otherOs = $state<Os>('windows')
  let guide = $state(false)

  const me = $derived(view?.me ?? null)
  const other = $derived(view?.other ?? null)
  const sel = $derived(rows.find((r) => r.node.path === selected)?.node ?? null)

  function fail(e: unknown) {
    error = String(e)
  }

  function say(t: string) {
    toast = t
    setTimeout(() => (toast === t ? (toast = null) : null), 3500)
  }

  async function loadRoot() {
    const kids = await api.children('')
    rows = kids.map((node) => ({ node, depth: 0, open: false, loading: false }))
  }

  /** Re-read every open folder, keeping what is open and what is selected. */
  async function reloadOpen() {
    const open = new Set(rows.filter((r) => r.open).map((r) => r.node.path))
    const build = async (path: string, depth: number): Promise<Row[]> => {
      const out: Row[] = []
      for (const node of await api.children(path)) {
        const isOpen = node.dir && open.has(node.path)
        out.push({ node, depth, open: isOpen, loading: false })
        if (isOpen) out.push(...(await build(node.path, depth + 1)))
      }
      return out
    }
    rows = await build('', 0)
  }

  async function boot(v: View) {
    view = v
    error = v.message
    if (v.stage === 'ready') await reloadOpen()
  }

  onMount(() => {
    api.state().then(boot).catch(fail)
  })

  async function toggle(n: NodeView) {
    const i = rows.findIndex((r) => r.node.path === n.path)
    if (i < 0) return
    const r = rows[i]
    if (r.open) {
      let j = i + 1
      while (j < rows.length && rows[j].depth > r.depth) j++
      rows.splice(i + 1, j - i - 1)
      r.open = false
      return
    }
    r.loading = true
    try {
      const kids = await api.children(n.path)
      const at = rows.findIndex((x) => x.node.path === n.path)
      rows.splice(at + 1, 0, ...kids.map((node) => ({ node, depth: r.depth + 1, open: false, loading: false })))
      rows[at].open = true
    } catch (e) {
      fail(e)
    } finally {
      r.loading = false
    }
  }

  function parentDir(n: NodeView): Dir {
    const i = n.path.lastIndexOf('/')
    if (i < 0) return dirOf(view?.rootMode ?? 'off', me?.id)
    const p = rows.find((r) => r.node.path === n.path.slice(0, i))
    return dirOf(p?.node.effective ?? 'off', me?.id)
  }

  /** A file can only take its own direction when its folder is off; otherwise off or the folder's. */
  function allowed(n: NodeView): Dir[] {
    if (n.dir) return ['off', 'out', 'both', 'in']
    const p = parentDir(n)
    return p === 'off' ? ['off', 'out', 'both', 'in'] : ['off', p]
  }

  async function pick(path: string, dir: boolean, d: Dir | null) {
    if (!me || !other) return
    try {
      view = await api.setRule(path, dir, d === null ? null : modeOf(d, me, other))
      await reloadOpen()
    } catch (e) {
      fail(e)
    }
  }

  async function allow(what: 'metered' | 'relay', on: boolean) {
    try {
      view = await api.setAllow(what, on)
    } catch (e) {
      fail(e)
    }
  }

  async function resume(folder: string) {
    try {
      view = await api.resume(folder)
      say('Folder resumed.')
    } catch (e) {
      fail(e)
    }
  }

  async function review() {
    guide = false
    try {
      plan = await api.plan()
    } catch (e) {
      fail(e)
    }
  }

  async function apply(guardConfirmed: boolean) {
    busy = true
    try {
      const r: Report = await api.apply(guardConfirmed)
      plan = null
      await boot(await api.refresh())
      say(`Applied. ${r.added} added, ${r.updated} changed, ${r.removed} removed.`)
    } catch (e) {
      fail(e)
    } finally {
      busy = false
    }
  }

  async function discard() {
    try {
      await boot(await api.discard())
      say('Changes discarded.')
    } catch (e) {
      fail(e)
    }
  }

  async function refresh() {
    try {
      await boot(await api.refresh())
    } catch (e) {
      fail(e)
    }
  }

  async function init() {
    try {
      await boot(await api.init(rootInput))
    } catch (e) {
      fail(e)
    }
  }

  async function createRules() {
    try {
      await boot(await api.createRules(otherOs))
    } catch (e) {
      fail(e)
    }
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === 'Escape' && plan) plan = null
    else if ((e.key === 'F5' || (e.ctrlKey && e.key === 'r')) && !plan) {
      e.preventDefault()
      refresh()
    }
  }

  const revState = $derived.by(() => {
    if (!view || view.sharedRevision === null) return null
    const want = view.sharedRevision
    const side = (r: number | null) => (r === want ? 'ok' : r === null ? 'none' : 'behind')
    return { want, me: side(me?.appliedRevision ?? null), other: side(other?.appliedRevision ?? null) }
  })
</script>

<svelte:window onkeydown={onkey} />

<div class="shell">
  <header class="top">
    <div class="brand">
      <img src="/favicon.svg" alt="" width="26" height="26" />
      <span class="word">Setu</span>
    </div>
    {#if view?.stage === 'ready' && me && other}
      <div class="link">
        <span class="dev">{me.name}<small>this computer</small></span>
        <span class="wire" class:up={other.connected} title={other.connected ? `Connected${other.address ? ' via ' + other.address : ''}` : 'Not connected'}>
          <span class="dot"></span>
        </span>
        <span class="dev">{other.name}<small>{other.connected ? 'connected' : 'not connected'}</small></span>
      </div>
      {#if revState}
        <div class="revs" title="Every change to the rules is a numbered revision. Both computers should show the same number.">
          <span>Rules r{revState.want}</span>
          <span class="rev {revState.me}">here {me.appliedRevision ? 'r' + me.appliedRevision : '—'}</span>
          <span class="rev {revState.other}">{other.name} {other.appliedRevision ? 'r' + other.appliedRevision : '—'}</span>
        </div>
      {/if}
    {/if}
    <div class="grow"></div>
    {#if view?.stage === 'ready'}
      <button class="btn" onclick={() => (guide = true)} title="Go through the top folders one at a time">First-sync guide</button>
      <button class="btn" onclick={refresh} title="Read the folders again (F5)">Refresh</button>
    {/if}
  </header>

  {#if error}
    <div class="banner bad" role="alert">
      <span>{error}</span>
      <button onclick={() => (error = null)} aria-label="Dismiss">✕</button>
    </div>
  {/if}
  {#if view?.conflicts.length}
    <div class="banner warn">
      The rules file was changed on both computers at once. Setu kept one; the other is saved as
      {view.conflicts.join(', ')}. Check your choices, then apply again.
    </div>
  {/if}

  {#each view?.watch?.holds ?? [] as h (h.folder)}
    <div class="banner bad" role="alert">
      <span>
        <b>{h.rel === null ? h.folder : h.rel || 'The whole folder'}</b> is paused here: {h.deletes} files were deleted on
        {other?.name ?? 'the other computer'} within two minutes. Check that this was meant. Deleted files are kept for 30 days in
        the folder's <span class="mono">.stversions</span>.
      </span>
      <button class="btn" onclick={() => resume(h.folder)}>Resume syncing</button>
    </div>
  {/each}
  {#if view?.stage === 'ready' && view.watch && view.watch.running && !view.watch.gateOpen}
    <div class="banner warn">
      <span>Syncing is on hold here: {view.watch.reason}. Setu's folders resume when the network is fine again.</span>
      {#if view.watch.metered && !view.watch.allowMetered}<button class="btn" onclick={() => allow('metered', true)}>Allow on metered network</button>{/if}
      {#if view.watch.relayed && !view.watch.allowRelay}<button class="btn" onclick={() => allow('relay', true)}>Allow through relay</button>{/if}
    </div>
  {/if}
  {#if view?.stage === 'ready' && view.watch && !view.watch.running}
    <div class="banner warn">
      <span>Setu's background helper is not running, so the network gate and the delete guard are off. It starts at logon once installed (<span class="mono">setu-watch</span>).</span>
    </div>
  {/if}

  {#if !view}
    <div class="center muted">Starting…</div>
  {:else if view.stage === 'setup'}
    <div class="center">
      <div class="card">
        <h1>Choose the folder to share</h1>
        <p class="muted">Pick the folder on this computer that Setu manages. Nothing syncs until you choose what goes where and press Apply.</p>
        <input type="text" placeholder="/home/you/Work or C:\Work" bind:value={rootInput} onkeydown={(e) => e.key === 'Enter' && init()} />
        <div class="row-end"><button class="btn primary" disabled={!rootInput.trim()} onclick={init}>Continue</button></div>
      </div>
    </div>
  {:else if view.stage === 'rules'}
    <div class="center">
      <div class="card">
        <h1>Start the rules</h1>
        <p class="muted">No rules yet. Do this on one computer only; the other picks them up through Syncthing. Everything starts Off.</p>
        <p class="muted mono">{view.root}</p>
        <label class="field">
          The other computer runs
          <select bind:value={otherOs}>
            <option value="windows">Windows</option>
            <option value="linux">Linux</option>
            <option value="macos">macOS</option>
          </select>
        </label>
        <div class="row-end"><button class="btn primary" onclick={createRules}>Create rules</button></div>
      </div>
    </div>
  {:else if me && other}
    <main class="main">
      <section class="treepane">
        <button class="rootrow" class:sel={selected === null} onclick={() => (selected = null)}>
          <span class="mono path">{view.root}</span>
          <span class="muted small">{view.localEntries.toLocaleString()} items here{view.remoteEntries !== null ? ` · ${view.remoteEntries.toLocaleString()} on ${other.name}` : ''}</span>
          <span class="chip {dirOf(view.rootMode, me.id)}">{shortLabel(dirOf(view.rootMode, me.id))}</span>
        </button>
        {#if view.remoteEntries === null}
          <p class="hint">{other.name}'s file list has not arrived yet. Open Setu there once; until then only this computer's files show.</p>
        {/if}
        <Tree {rows} {selected} meId={me.id} thereName={other.name} remoteKnown={view.remoteEntries !== null} onselect={(n) => (selected = n.path)} ontoggle={toggle} />
      </section>

      <aside class="details">
        {#if sel}
          {@const d = dirOf(sel.effective, me.id)}
          <h2 class="title" title={sel.path}>{sel.name}</h2>
          <p class="mono muted crumb">{sel.path}</p>
          <DirPicker value={d} allowed={allowed(sel)} meName={me.name} otherName={other.name} onpick={(x) => pick(sel.path, sel.dir, x)} />
          <p class="say">{sentence(d, me.name, other.name)}</p>
          <p class="muted small">
            {#if sel.explicit !== null}
              Set on this {sel.dir ? 'folder' : 'file'}.
              <button class="link-btn" onclick={() => pick(sel.path, sel.dir, null)}>Use the folder's choice</button>
            {:else}
              Inherited from {sel.from ? sel.from : 'the whole folder'}.
            {/if}
          </p>
          {#if !sel.dir && parentDir(sel) !== 'off'}
            <p class="muted small">To give this file its own direction, turn its folder Off first.</p>
          {/if}
          <dl class="facts">
            <dt>{me.name}</dt>
            <dd>{sel.here ? `${bytes(sel.here.size)}${sel.dir ? ` · ${sel.here.files.toLocaleString()} files` : ''}` : 'not here'}</dd>
            <dt>{other.name}</dt>
            <dd>{sel.there ? `${bytes(sel.there.size)}${sel.dir ? ` · ${sel.there.files.toLocaleString()} files` : ''}` : view.remoteEntries === null ? 'list not arrived' : 'not there'}</dd>
          </dl>
        {:else}
          {@const d = dirOf(view.rootMode, me.id)}
          <h2 class="title">Whole folder</h2>
          <p class="mono muted crumb">{view.root}</p>
          <DirPicker value={d} meName={me.name} otherName={other.name} onpick={(x) => pick('', true, x)} />
          <p class="say">{sentence(d, me.name, other.name)}</p>
          <p class="muted small">Everything inside follows this unless you choose otherwise for a folder or file.</p>
          <div class="never">
            <h3>Never synced</h3>
            <p class="muted small">Secrets and build output stay on each computer, whatever you choose.</p>
            <p class="mono small">API keys · .env · keystores · {view.alwaysIgnore.map((x) => x.replace('(?d)', '')).join(' · ')}</p>
          </div>
          <p class="muted small">{other.name}'s file list: {ago(view.remoteListAge)}</p>
          {#if view.watch}
            <div class="never">
              <h3>Network</h3>
              <label class="tog"><input type="checkbox" checked={view.watch.allowMetered} onchange={(e) => allow('metered', (e.currentTarget as HTMLInputElement).checked)} /> Sync on metered networks</label>
              <label class="tog"><input type="checkbox" checked={view.watch.allowRelay} onchange={(e) => allow('relay', (e.currentTarget as HTMLInputElement).checked)} /> Sync through a relay (slower, uses more data)</label>
              <p class="muted small">Now: {view.watch.running ? `${view.watch.metered ? 'metered' : 'not metered'}, ${view.watch.relayed ? 'relayed link' : 'direct link'}` : 'helper not running'}</p>
            </div>
          {/if}
        {/if}
      </aside>
    </main>

    <footer class="bar" class:live={view.edits.length > 0}>
      {#if view.edits.length}
        <span><b>{view.edits.length}</b> change{view.edits.length === 1 ? '' : 's'} not applied yet</span>
        <div class="grow"></div>
        <button class="btn" onclick={discard}>Discard</button>
        <button class="btn primary" onclick={review}>Review and apply</button>
      {:else}
        <span class="muted">
          {revState?.me === 'ok' ? 'This computer matches the rules.' : 'This computer has not applied the latest rules.'}
        </span>
        <div class="grow"></div>
        {#if revState?.me !== 'ok'}<button class="btn primary" onclick={review}>Review and apply</button>{/if}
      {/if}
    </footer>
  {/if}

  {#if toast}<div class="toast" role="status">{toast}</div>{/if}
  {#if guide && me && other && view}
    <FirstSync
      folders={rows.filter((r) => r.depth === 0 && r.node.dir).map((r) => r.node)}
      {me}
      {other}
      remoteKnown={view.remoteEntries !== null}
      onpick={(n, d) => pick(n.path, true, d)}
      onclose={() => (guide = false)}
      onreview={review}
    />
  {/if}
  {#if plan && me && other}
    <PlanSheet view={plan} {me} {other} {busy} onclose={() => (plan = null)} onapply={apply} />
  {/if}
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 18px;
    height: 56px;
    padding: 0 16px;
    background: var(--surface);
    border-bottom: 1px solid var(--line);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .word {
    font-weight: 750;
    font-size: 16px;
    letter-spacing: -0.01em;
  }
  .link {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 12px;
    border-radius: 99px;
    background: var(--surface-2);
  }
  .dev {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
    font-size: 13px;
    line-height: 1.15;
  }
  .dev small {
    display: block;
    font-weight: 450;
    font-size: 11px;
    color: var(--muted);
  }
  .wire {
    position: relative;
    width: 46px;
    height: 2px;
    background: var(--line);
    border-radius: 2px;
  }
  .wire.up {
    background: linear-gradient(90deg, var(--out), var(--in));
  }
  .wire .dot {
    position: absolute;
    top: -3px;
    left: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--faint);
  }
  .wire.up .dot {
    background: var(--accent);
    animation: travel 2.4s ease-in-out infinite;
  }
  @keyframes travel {
    50% {
      left: 38px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wire.up .dot {
      animation: none;
      left: 19px;
    }
  }
  .revs {
    display: flex;
    gap: 6px;
    font-size: 12px;
    color: var(--muted);
    align-items: center;
  }
  .rev {
    padding: 2px 8px;
    border-radius: 99px;
    font-weight: 600;
  }
  .rev.ok {
    color: var(--ok);
    background: color-mix(in srgb, var(--ok) 13%, transparent);
  }
  .rev.behind,
  .rev.none {
    color: var(--warn);
    background: color-mix(in srgb, var(--warn) 15%, transparent);
  }
  .grow {
    flex: 1;
  }
  .banner {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 10px 16px;
    font-size: 13px;
  }
  .banner span {
    flex: 1;
  }
  .banner.bad {
    background: var(--bad-bg);
    color: var(--bad);
  }
  .banner.warn {
    background: var(--warn-bg);
    color: var(--warn);
  }
  .center {
    flex: 1;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .card {
    width: min(520px, 100%);
    padding: 28px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 16px;
    box-shadow: var(--shadow);
  }
  .card h1 {
    margin: 0 0 6px;
    font-size: 20px;
  }
  .card p {
    margin: 0 0 16px;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }
  select {
    font: inherit;
    height: 34px;
    padding: 0 8px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--text);
  }
  .row-end {
    display: flex;
    justify-content: flex-end;
    margin-top: 16px;
  }
  .main {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 340px;
  }
  @media (max-width: 900px) {
    .main {
      grid-template-columns: minmax(0, 1fr) 260px;
    }
    .revs > span:first-child,
    .rootrow .small {
      display: none;
    }
  }
  .treepane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 10px 8px 0 12px;
  }
  .rootrow {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 12px;
    margin-bottom: 6px;
    border-radius: 10px;
    border: 1px solid var(--line);
    background: var(--surface);
    text-align: left;
  }
  .rootrow.sel {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .rootrow .path {
    font-weight: 600;
  }
  .rootrow .small {
    flex: 1;
  }
  .hint {
    margin: 0 0 6px;
    padding: 8px 12px;
    font-size: 12.5px;
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--muted);
  }
  .chip {
    font-size: 12px;
    font-weight: 600;
    padding: 2px 9px;
    border-radius: 99px;
    background: color-mix(in srgb, currentColor 12%, transparent);
  }
  .chip.off {
    color: var(--off);
  }
  .chip.both {
    color: var(--both);
  }
  .chip.out {
    color: var(--out);
  }
  .chip.in {
    color: var(--in);
  }
  .details {
    overflow: auto;
    padding: 20px;
    border-left: 1px solid var(--line);
    background: var(--surface);
  }
  .title {
    margin: 0;
    font-size: 18px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .crumb {
    margin: 2px 0 16px;
    overflow-wrap: anywhere;
  }
  .say {
    margin: 14px 0 6px;
    line-height: 1.45;
  }
  .small {
    font-size: 12.5px;
  }
  .link-btn {
    color: var(--accent);
    font-weight: 600;
    padding: 0;
  }
  .link-btn:hover {
    text-decoration: underline;
  }
  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 14px;
    margin: 18px 0 0;
    padding-top: 14px;
    border-top: 1px solid var(--line);
    font-size: 13px;
  }
  .facts dt {
    color: var(--muted);
  }
  .facts dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
  .never {
    margin: 18px 0 12px;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--surface-2);
  }
  .never h3 {
    margin: 0 0 4px;
    font-size: 13px;
  }
  .never p {
    margin: 4px 0 0;
    overflow-wrap: anywhere;
  }
  .tog {
    display: flex;
    gap: 8px;
    align-items: center;
    margin: 6px 0;
    font-size: 13px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 56px;
    padding: 0 16px;
    border-top: 1px solid var(--line);
    background: var(--surface);
  }
  .bar.live {
    background: color-mix(in srgb, var(--accent) 7%, var(--surface));
  }
  .toast {
    position: fixed;
    bottom: 72px;
    left: 50%;
    transform: translateX(-50%);
    padding: 10px 16px;
    border-radius: 10px;
    background: var(--text);
    color: var(--bg);
    font-size: 13px;
    box-shadow: var(--shadow);
  }
</style>
