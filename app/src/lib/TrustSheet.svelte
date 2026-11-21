<script lang="ts" module>
  export type TrustTab = 'conflicts' | 'versions' | 'check'
</script>

<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type Conflict, type DeviceView, type FolderCheck, type Kept } from './api'
  import { bytes } from './format'

  let {
    tab = $bindable(),
    me,
    other,
    onclose,
    onchanged,
  }: {
    tab: TrustTab
    me: DeviceView
    other: DeviceView
    onclose: () => void
    /** Something on disk changed: the tree should reload. */
    onchanged: (message: string) => void
  } = $props()

  let conflicts = $state<Conflict[] | null>(null)
  let versions = $state<Kept[] | null>(null)
  let checks = $state<FolderCheck[] | null>(null)
  let loading = $state(false)
  let error = $state<string | null>(null)
  let filter = $state('')
  let working = $state<string | null>(null)

  const when = (secs: number | null) => (secs === null ? '' : new Date(secs * 1000).toLocaleString())
  const whenIso = (iso: string) => new Date(iso).toLocaleString()
  const nameOf = (p: string) => p.slice(p.lastIndexOf('/') + 1)
  const dirName = (p: string) => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : '')

  async function load(t: TrustTab, rescan = false) {
    loading = true
    error = null
    try {
      if (t === 'conflicts') conflicts = await api.conflicts()
      else if (t === 'versions') versions = await api.versions()
      else checks = await api.verify(rescan)
    } catch (e) {
      error = String(e)
    } finally {
      loading = false
    }
  }

  function show(t: TrustTab) {
    tab = t
    const have = t === 'conflicts' ? conflicts : t === 'versions' ? versions : checks
    if (have === null) load(t)
  }

  onMount(() => load(tab))

  async function keep(c: Conflict, keepCopy: boolean) {
    working = c.copy
    error = null
    try {
      conflicts = await api.keep(c.copy, keepCopy)
      versions = null
      onchanged(`Kept the ${keepCopy ? 'conflict copy' : 'current file'} of ${nameOf(c.original)}. The other one is in .stversions.`)
    } catch (e) {
      error = String(e)
    } finally {
      working = null
    }
  }

  async function restore(k: Kept) {
    const key = k.folder + k.path + k.versionTime
    working = key
    error = null
    try {
      const r = await api.restore(k.folder, k.path, k.versionTime)
      versions = await api.versions()
      onchanged(
        r === 'inPlace'
          ? `Restored ${nameOf(k.rel)}. The file it replaced is kept as a version.`
          : `${me.name} only receives this folder, so the old copy was saved beside it as ${nameOf(r.beside)}.`,
      )
    } catch (e) {
      error = String(e)
    } finally {
      working = null
    }
  }

  const shown = $derived.by(() => {
    if (!versions) return []
    const f = filter.trim().toLowerCase()
    const list = f ? versions.filter((v) => v.rel.toLowerCase().includes(f)) : versions
    return list.slice(0, 500)
  })

  const allOk = $derived(checks !== null && checks.length > 0 && checks.every((c) => c.ok))
  const typeLabel = (t: string) => ({ sendonly: 'sends', receiveonly: 'receives', sendreceive: 'both ways' })[t] ?? t
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-label="Conflicts, versions and check">
  <div class="tabs" role="tablist" tabindex="-1">
    <button role="tab" aria-selected={tab === 'conflicts'} class:on={tab === 'conflicts'} onclick={() => show('conflicts')}>
      Conflicts{#if conflicts?.length}<span class="count">{conflicts.length}</span>{/if}
    </button>
    <button role="tab" aria-selected={tab === 'versions'} class:on={tab === 'versions'} onclick={() => show('versions')}>Old versions</button>
    <button role="tab" aria-selected={tab === 'check'} class:on={tab === 'check'} onclick={() => show('check')}>Check</button>
    <div class="grow"></div>
    <button class="close" onclick={onclose} aria-label="Close">✕</button>
  </div>

  <div class="body">
    {#if error}<p class="box bad">{error}</p>{/if}

    {#if tab === 'conflicts'}
      <p class="muted lead">
        When a file changed on both computers, Syncthing keeps both: the newer one under the normal name, and the other as a
        <span class="mono">.sync-conflict-</span> copy. Pick the one to keep. The other goes to the folder's
        <span class="mono">.stversions</span>; nothing is deleted.
      </p>
      {#if conflicts === null}
        <p class="muted">Looking…</p>
      {:else if conflicts.length === 0}
        <p class="empty">No conflict copies. Every synced file has one version.</p>
      {:else}
        <ul class="list">
          {#each conflicts as c (c.copy)}
            <li class="conflict">
              <div class="name">
                <b>{nameOf(c.original)}</b>
                <span class="mono muted small">{dirName(c.original) || '(top folder)'}</span>
              </div>
              <div class="pair">
                <div class="side">
                  <span class="label">Current file</span>
                  {#if c.originalSize !== null}
                    <span>{bytes(c.originalSize)} · {when(c.originalMtime)}</span>
                    <button class="btn" disabled={working !== null} onclick={() => keep(c, false)}>Keep this</button>
                  {:else}
                    <span class="muted">gone</span>
                  {/if}
                </div>
                <div class="side">
                  <span class="label">Conflict copy</span>
                  <span>{bytes(c.copySize)} · {when(c.copyMtime)}</span>
                  <button class="btn" disabled={working !== null} onclick={() => keep(c, true)}>Keep this</button>
                </div>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if tab === 'versions'}
      <p class="muted lead">
        Each Setu folder keeps replaced and deleted files for 30 days. Restoring puts the old copy back; the file it replaces
        becomes a version itself. In a folder {me.name} only receives, the old copy is saved beside the file instead, so
        {other.name} does not overwrite it.
      </p>
      <input type="search" placeholder="Filter by path" bind:value={filter} />
      {#if versions === null}
        <p class="muted">Looking…</p>
      {:else if versions.length === 0}
        <p class="empty">No old versions kept yet.</p>
      {:else}
        <ul class="list">
          {#each shown as k (k.folder + k.path + k.versionTime)}
            <li class="row">
              <div class="name">
                <b>{nameOf(k.rel)}</b>
                <span class="mono muted small">{dirName(k.rel) || '(top folder)'}</span>
              </div>
              <span class="small muted nowrap" title="Last changed {whenIso(k.modTime)}">kept {whenIso(k.versionTime)}</span>
              <span class="small nowrap">{bytes(k.size)}</span>
              <button class="btn" disabled={working !== null} onclick={() => restore(k)}>
                {working === k.folder + k.path + k.versionTime ? 'Restoring…' : 'Restore'}
              </button>
            </li>
          {/each}
        </ul>
        {#if shown.length < (filter ? versions.filter((v) => v.rel.toLowerCase().includes(filter.trim().toLowerCase())).length : versions.length)}
          <p class="muted small">Showing the newest 500. Filter to find older ones.</p>
        {/if}
      {/if}
    {:else}
      <p class="muted lead">
        Syncthing compares every file block by block (SHA-256). A folder is complete here when it is idle, needs nothing from
        {other.name}, and has no errors. The other computer's own view is shown beside it.
      </p>
      {#if checks === null}
        <p class="muted">Checking…</p>
      {:else if checks.length === 0}
        <p class="empty">No folders sync yet. Choose what goes where and press Apply.</p>
      {:else}
        {#if allOk}<p class="box okbox">All {checks.length} folders are complete on {me.name}.</p>{/if}
        <ul class="list">
          {#each checks as c (c.id)}
            <li class="check">
              <span class="mark" class:ok={c.ok}>{c.ok ? '✓' : '!'}</span>
              <div class="name">
                <b>{c.rel || 'Whole folder'}</b>
                <span class="small muted">
                  {typeLabel(c.folderType)} · {c.state}
                  {#if c.needFiles}· needs {c.needFiles.toLocaleString()} files ({bytes(c.needBytes)}){/if}
                  {#if c.otherCompletion !== null}· {other.name} {Math.floor(c.otherCompletion)} %{/if}
                </span>
                {#if c.localChanges}
                  <span class="small warn">{c.localChanges} files changed here are not sent back ({me.name} only receives)</span>
                {/if}
                {#each c.errors.slice(0, 5) as e, i (i)}<span class="small bad-text mono">{e}</span>{/each}
                {#each c.samples.slice(0, 5) as p (p)}<span class="small muted mono">waiting: {p}</span>{/each}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>

  <footer>
    {#if tab === 'check'}
      <button class="btn" disabled={loading} onclick={() => load('check', true)} title="Ask Syncthing to look at every folder again first">
        {loading ? 'Checking…' : 'Scan again and check'}
      </button>
    {:else}
      <button class="btn" disabled={loading} onclick={() => load(tab)}>{loading ? 'Looking…' : 'Look again'}</button>
    {/if}
    <button class="btn primary" onclick={onclose}>Done</button>
  </footer>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgb(10 12 20 / 45%);
    animation: fade 160ms ease-out;
  }
  .sheet {
    position: fixed;
    top: 50%;
    left: 50%;
    width: min(760px, calc(100vw - 32px));
    height: min(640px, calc(100vh - 64px));
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 16px;
    box-shadow: var(--shadow);
    animation: rise 200ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 12px 16px 0;
    border-bottom: 1px solid var(--line);
  }
  .tabs [role='tab'] {
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    font-weight: 600;
    padding: 9px 12px;
    border-bottom: 2px solid transparent;
    cursor: pointer;
  }
  .tabs [role='tab'].on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .count {
    margin-left: 6px;
    padding: 1px 7px;
    border-radius: 99px;
    font-size: 11.5px;
    background: var(--warn-bg);
    color: var(--warn);
  }
  .grow {
    flex: 1;
  }
  .close {
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 15px;
    cursor: pointer;
    padding: 6px 8px;
  }
  .body {
    flex: 1;
    padding: 12px 24px;
    overflow: auto;
  }
  .lead {
    margin: 4px 0 12px;
    font-size: 13px;
  }
  .empty {
    margin: 40px 0;
    text-align: center;
    color: var(--muted);
  }
  input[type='search'] {
    width: 100%;
    margin-bottom: 8px;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .list li {
    border-bottom: 1px solid var(--line);
    padding: 10px 0;
    min-width: 0;
  }
  .row,
  .check {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .check {
    align-items: flex-start;
  }
  .name {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name > * {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    margin-top: 8px;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: flex-start;
    padding: 10px 12px;
    border: 1px solid var(--line);
    border-radius: 10px;
    font-size: 13px;
  }
  .label {
    font-size: 11.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--faint);
  }
  .mark {
    flex: none;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 12px;
    font-weight: 700;
    background: var(--warn-bg);
    color: var(--warn);
  }
  .mark.ok {
    background: color-mix(in srgb, var(--ok) 15%, transparent);
    color: var(--ok);
  }
  .small {
    font-size: 12.5px;
  }
  .nowrap {
    white-space: nowrap;
  }
  .warn {
    color: var(--warn);
  }
  .bad-text {
    color: var(--bad);
  }
  .box {
    margin: 0 0 10px;
    padding: 10px 14px;
    border-radius: 10px;
  }
  .bad {
    background: var(--bad-bg);
    color: var(--bad);
  }
  .okbox {
    background: color-mix(in srgb, var(--ok) 12%, transparent);
    color: var(--ok);
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 14px 24px 18px;
    border-top: 1px solid var(--line);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translate(-50%, calc(-50% + 12px)) scale(0.98);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .scrim,
    .sheet {
      animation: none;
    }
  }
  @media (max-width: 560px) {
    .pair {
      grid-template-columns: 1fr;
    }
  }
</style>
