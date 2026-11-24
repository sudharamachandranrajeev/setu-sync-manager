<script lang="ts" module>
  export type PauseHow = 'm15' | 'hour' | 'h3' | 'tomorrow' | 'manual'
  export const pauseChoices: { how: PauseHow; label: string }[] = [
    { how: 'm15', label: 'For 15 minutes' },
    { how: 'hour', label: 'For 1 hour' },
    { how: 'h3', label: 'For 3 hours' },
    { how: 'tomorrow', label: 'Until tomorrow morning' },
    { how: 'manual', label: 'Until I resume' },
  ]
</script>

<script lang="ts">
  import { api, type Flow, type FolderProgress, type Snapshot } from './api'
  import { bytes, clock, duration, rate, stateWord } from './format'

  let {
    snap,
    there,
    pausedText,
    onpause,
    onclose,
    onchanged,
  }: {
    snap: Snapshot | null
    /** Everyday name of the other computer. */
    there: string
    /** "until 14:00" when the person paused this computer, else null. */
    pausedText: string | null
    onpause: (how: PauseHow | null) => void
    onclose: () => void
    onchanged: (message: string) => void
  } = $props()

  let menu = $state(false)
  let working = $state<string | null>(null)
  let error = $state<string | null>(null)

  const online = $derived(snap?.peers.some((p) => p.connected) ?? false)

  async function folderPause(f: FolderProgress, pause: boolean) {
    working = f.id
    error = null
    try {
      await api.pauseFolder(f.id, pause)
      onchanged(pause ? `${f.name} paused on this computer.` : `${f.name} resumed.`)
    } catch (e) {
      error = String(e)
    } finally {
      working = null
    }
  }

  async function rescan() {
    working = 'rescan'
    error = null
    try {
      await api.verify(true)
      onchanged('Looking through every synced folder for changes.')
    } catch (e) {
      error = String(e)
    } finally {
      working = null
    }
  }

  async function show(f: FolderProgress) {
    try {
      await api.openPath(f.path)
    } catch (e) {
      error = String(e)
    }
  }

  /** How much of a folder is complete on this computer. */
  const here = (f: FolderProgress) => (f.globalBytes ? ((f.globalBytes - Math.min(f.needBytes - f.partialBytes, f.globalBytes)) * 100) / f.globalBytes : 100)
  const pct = (n: number) => (n >= 99.95 && n < 100 ? '99.9' : n.toFixed(n >= 10 ? 0 : 1))
  const why = { you: 'paused by you', safety: 'paused for safety: many files were deleted on the other computer', hold: 'on hold with the whole computer' }

  /** What a direction is doing, in a few words. */
  function headline(f: Flow, toHere: boolean): string {
    if (!f.active) return 'Nothing to copy'
    if (snap?.paused) return 'Paused'
    if (!online) return `Waiting for ${there} to come online`
    if (f.speed === 0) return toHere ? 'Getting ready to copy' : `Waiting for ${there} to ask for files`
    return 'Copying'
  }
</script>

{#snippet flowCard(title: string, f: Flow, toHere: boolean)}
  {@const last = snap?.lastDone && snap.lastDone.toHere === toHere ? snap.lastDone : null}
  <section class="flow" class:active={f.active}>
    <div class="fhead">
      <h3>{title}</h3>
      <span class="state">{headline(f, toHere)}</span>
    </div>
    {#if f.active}
      <div class="big">
        <span class="pct">{pct(f.percent)}<small>%</small></span>
        <span class="muted">{bytes(f.doneBytes)} of {bytes(f.totalBytes)}</span>
      </div>
      <div class="bar" role="progressbar" aria-label="{title}: copied" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.floor(f.percent)}>
        <span style="width:{Math.min(100, f.percent)}%" class:paused={snap?.paused}></span>
      </div>
      <dl>
        <dt>Speed</dt>
        <dd>{snap?.paused ? 'paused' : rate(f.speed)} <span class="muted">· average {rate(f.average)}</span></dd>
        <dt>Time so far</dt>
        <dd>{duration(f.elapsedSecs)}{#if f.startedAt}<span class="muted"> · started {clock(f.startedAt)}</span>{/if}</dd>
        <dt>Time left</dt>
        <dd>
          {#if snap?.paused}waits until you resume
          {:else if f.etaSecs !== null}about {duration(f.etaSecs)} <span class="muted">· done around {clock((snap?.at ?? 0) + f.etaSecs)}</span>
          {:else}working it out…{/if}
        </dd>
        <dt>Left</dt>
        <dd>{bytes(f.leftBytes)} <span class="muted">· {f.leftFiles.toLocaleString()} file{f.leftFiles === 1 ? '' : 's'}</span></dd>
      </dl>
    {:else if last}
      <p class="muted small">Last copied {bytes(last.bytes)} ({last.files.toLocaleString()} files) in {duration(last.secs)}, finished {clock(last.at)}.</p>
    {:else}
      <p class="muted small">Everything that should be here {toHere ? 'is here' : `is on ${there}`}.</p>
    {/if}
  </section>
{/snippet}

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-labelledby="pg-title">
  <header>
    <h2 id="pg-title">Copy progress</h2>
    <span class="muted small live">{snap ? `Updated ${clock(snap.at)}, every few seconds` : 'Reading…'}</span>
    <div class="grow"></div>
    <button class="close" onclick={onclose} aria-label="Close">✕</button>
  </header>

  <div class="body">
    {#if error}<p class="box bad">{error}</p>{/if}
    {#if pausedText}
      <div class="box warn row">
        <span>Syncing is paused on this computer {pausedText}. Nothing is copied to or from it until then.</span>
        <button class="btn" onclick={() => onpause(null)}>Resume now</button>
      </div>
    {:else if snap?.paused}
      <p class="box warn">Copying is on hold: the network is not suitable right now (see Network under Everything). It starts again by itself.</p>
    {/if}

    {#if !snap}
      <p class="muted">Reading progress from Syncthing…</p>
    {:else}
      <div class="flows">
        {@render flowCard('To this computer', snap.down, true)}
        {@render flowCard(`To ${there}`, snap.up, false)}
      </div>

      {#if snap.peers.length}
        <p class="peers small">
          {#each snap.peers as p (p.id)}
            <span class="peer"><span class="dot" class:on={p.connected}></span>{p.name}: {p.connected ? `connected${p.address ? ` (${p.address})` : ''}` : 'offline'}</span>
          {/each}
        </p>
      {/if}

      <h3 class="section">Folders</h3>
      {#if snap.folders.length === 0}
        <p class="empty">Nothing syncs yet. Use Quick setup or choose folders in the list, then press Start syncing.</p>
      {:else}
        <ul class="folders">
          {#each snap.folders as f (f.id)}
            {@const h = here(f)}
            <li>
              <div class="frow">
                <b class="fname" title={f.path}>{f.name}</b>
                <span class="small muted">{f.paused && f.pausedBy ? why[f.pausedBy] : stateWord(f.state)}</span>
                <div class="grow"></div>
                <button class="link-btn small" onclick={() => show(f)} title="Show this folder in the file manager">Open</button>
                {#if f.paused && f.pausedBy === 'you'}
                  <button class="btn sm" disabled={working !== null} onclick={() => folderPause(f, false)}>Resume</button>
                {:else if !f.paused}
                  <button class="btn sm" disabled={working !== null} onclick={() => folderPause(f, true)} title="Stop copying this folder on this computer">Pause</button>
                {/if}
              </div>
              <div class="pair">
                <span class="small">Here</span>
                <div class="bar thin"><span style="width:{h}%" class:paused={f.paused}></span></div>
                <span class="small num">{pct(h)}%</span>
              </div>
              {#if f.otherCompletion !== null}
                <div class="pair">
                  <span class="small">{there}</span>
                  <div class="bar thin"><span style="width:{f.otherCompletion}%" class:paused={f.paused}></span></div>
                  <span class="small num">{pct(f.otherCompletion)}%</span>
                </div>
              {/if}
              <span class="small muted">
                {bytes(f.globalBytes)} · {f.globalFiles.toLocaleString()} files
                {#if f.needFiles}· {f.needFiles.toLocaleString()} files ({bytes(f.needBytes)}) still to come here{/if}
                {#if f.otherNeedFiles}· {f.otherNeedFiles.toLocaleString()} files ({bytes(f.otherNeedBytes)}) still to go to {there}{/if}
              </span>
              {#each f.copying.slice(0, 3) as c (c)}<span class="small mono muted copying">copying {c}</span>{/each}
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>

  <footer>
    <button class="btn" disabled={working !== null} onclick={rescan} title="Look through every synced folder for changes now, instead of waiting">
      {working === 'rescan' ? 'Looking…' : 'Look for changes now'}
    </button>
    <div class="grow"></div>
    {#if pausedText}
      <button class="btn" onclick={() => onpause(null)}>Resume syncing</button>
    {:else}
      <div class="pausewrap">
        <button class="btn" onclick={() => (menu = !menu)} aria-expanded={menu}>Pause syncing…</button>
        {#if menu}
          <div class="menu up" role="menu">
            {#each pauseChoices as c (c.how)}
              <button role="menuitem" onclick={() => ((menu = false), onpause(c.how))}>{c.label}</button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
    <button class="btn primary" onclick={onclose}>Done</button>
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
    width: min(820px, calc(100vw - 24px));
    max-height: calc(100vh - 24px);
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 16px;
    box-shadow: var(--shadow);
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 16px 20px 10px;
    border-bottom: 1px solid var(--line);
  }
  h2 {
    margin: 0;
    font-size: 18px;
  }
  .live {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .close {
    flex: none;
    width: 30px;
    height: 30px;
    border-radius: 8px;
  }
  .close:hover {
    background: var(--surface-2);
  }
  .body {
    padding: 14px 20px;
    overflow: auto;
    min-height: 0;
  }
  .grow {
    flex: 1;
  }
  .flows {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 12px;
  }
  .flow {
    padding: 14px 16px;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--surface-2);
  }
  .flow.active {
    background: var(--surface);
    border-color: color-mix(in srgb, var(--accent) 35%, var(--line));
  }
  .fhead {
    display: flex;
    align-items: baseline;
    gap: 10px;
    justify-content: space-between;
  }
  h3 {
    margin: 0;
    font-size: 14px;
  }
  .state {
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    text-align: right;
  }
  .big {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 8px 0 6px;
    flex-wrap: wrap;
  }
  .pct {
    font-size: 30px;
    font-weight: 750;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
  }
  .pct small {
    font-size: 16px;
    font-weight: 600;
  }
  .bar {
    height: 8px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-2));
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 600ms ease;
  }
  .bar span.paused {
    background: var(--warn);
  }
  .bar.thin {
    height: 5px;
    flex: 1;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 5px 14px;
    margin: 12px 0 0;
    font-size: 13px;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
  .peers {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
    margin: 12px 2px 0;
  }
  .peer {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--faint);
  }
  .dot.on {
    background: var(--ok);
  }
  .section {
    margin: 18px 0 8px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--faint);
  }
  .folders {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .folders li {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 0;
    border-bottom: 1px solid var(--line);
  }
  .frow {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .fname {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pair {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .pair > .small:first-child {
    width: 110px;
    flex: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }
  .num {
    width: 48px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .copying {
    overflow-wrap: anywhere;
  }
  .btn.sm {
    height: 28px;
    padding: 0 10px;
    font-size: 12.5px;
  }
  .link-btn {
    color: var(--accent);
    font-weight: 600;
    padding: 0;
  }
  .small {
    font-size: 12.5px;
  }
  .empty {
    color: var(--muted);
  }
  .box {
    margin: 0 0 12px;
    padding: 10px 14px;
    border-radius: 10px;
  }
  .box.row {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .box.row span {
    flex: 1;
  }
  .bad {
    background: var(--bad-bg);
    color: var(--bad);
  }
  .warn {
    background: var(--warn-bg);
    color: var(--warn);
  }
  footer {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 12px 20px 14px;
    border-top: 1px solid var(--line);
  }
  .pausewrap {
    position: relative;
  }
  .menu {
    position: absolute;
    right: 0;
    z-index: 5;
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 4px;
    border-radius: 10px;
    border: 1px solid var(--line);
    background: var(--surface);
    box-shadow: var(--shadow);
  }
  .menu.up {
    bottom: calc(100% + 6px);
  }
  .menu button {
    padding: 8px 10px;
    border-radius: 7px;
    text-align: left;
    white-space: nowrap;
  }
  .menu button:hover {
    background: var(--surface-2);
  }
  @media (prefers-reduced-motion: reduce) {
    .bar span {
      transition: none;
    }
  }
</style>
