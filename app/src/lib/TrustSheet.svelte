<script lang="ts" module>
  export type TrustTab = 'conflicts' | 'versions' | 'check' | 'same' | 'duplicates'
</script>

<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { api, type AccuracyRow, type Conflict, type FolderCheck, type Kept, type Mismatch, type Removed } from './api'
  import { bytes, stateWord } from './format'

  let {
    tab = $bindable(),
    there,
    onclose,
    onchanged,
  }: {
    tab: TrustTab
    /** Everyday name of the other computer. */
    there: string
    onclose: () => void
    /** Something on disk changed: the tree should reload. */
    onchanged: (message: string) => void
  } = $props()

  let conflicts = $state<Conflict[] | null>(null)
  let versions = $state<Kept[] | null>(null)
  let checks = $state<FolderCheck[] | null>(null)
  let same = $state<AccuracyRow[] | null>(null)
  let removed = $state<Removed[] | null>(null)
  /** Open list of differing files: computer id + top folder, and the list (null while on the way). */
  let diff = $state<{ key: string; name: string; top: string; list: Mismatch[] | null } | null>(null)
  let asked = $state(false)
  let timer: ReturnType<typeof setInterval> | null = null
  onDestroy(() => timer && clearInterval(timer))
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
      else if (t === 'same') same = await api.accuracy()
      else if (t === 'duplicates') removed = await api.tidyLog()
      else checks = await api.verify(rescan)
    } catch (e) {
      error = String(e)
    } finally {
      loading = false
    }
  }

  function show(t: TrustTab) {
    tab = t
    const have = { conflicts, versions, check: checks, same, duplicates: removed }[t]
    if (have === null) load(t)
  }

  onMount(() => load(tab))

  async function keep(c: Conflict, keepCopy: boolean) {
    working = c.copy
    error = null
    try {
      conflicts = await api.keep(c.copy, keepCopy)
      versions = null
      onchanged(`Kept the ${keepCopy ? 'other version' : 'current version'} of ${nameOf(c.original)}. The one you didn't pick is under Old versions.`)
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
          ? `Put back ${nameOf(k.rel)}. The version it replaced is kept under Old versions.`
          : `This folder only receives from ${there}, so the old copy was saved next to the file as ${nameOf(r.beside)}.`,
      )
    } catch (e) {
      error = String(e)
    } finally {
      working = null
    }
  }

  /** Ask the background helper to compare now; it takes a while on big folders, so look again every few seconds. */
  async function checkNow() {
    error = null
    try {
      await api.checkNow()
      asked = true
      const before = Math.max(0, ...(same ?? []).map((r) => r.checkedAt))
      if (timer) clearInterval(timer)
      timer = setInterval(async () => {
        same = await api.accuracy()
        if (Math.max(0, ...same.map((r) => r.checkedAt)) > before && timer) {
          clearInterval(timer)
          timer = null
          asked = false
        }
      }, 4000)
    } catch (e) {
      error = String(e)
    }
  }

  async function showDiff(id: string, name: string, top: string) {
    diff = { key: id + top, name, top, list: null }
    try {
      diff.list = (await api.differences(id, top)) ?? null
    } catch (e) {
      error = String(e)
    }
  }

  async function tidyNow() {
    working = 'tidy'
    error = null
    try {
      const r = await api.tidyNow()
      removed = await api.tidyLog()
      onchanged(r.length ? `Removed ${r.length} duplicate cop${r.length === 1 ? 'y' : 'ies'} (${bytes(r.reduce((s, x) => s + x.size, 0))}).` : 'No duplicate copies found.')
    } catch (e) {
      error = String(e)
    } finally {
      working = null
    }
  }

  async function undoTidy(r: Removed) {
    working = r.removed
    error = null
    try {
      removed = await api.undoTidy(r.removed)
      onchanged(`Put back ${nameOf(r.removed)}.`)
    } catch (e) {
      error = String(e)
    } finally {
      working = null
    }
  }

  const sameWord = { same: 'same files', differs: 'different', syncing: 'still copying', unchecked: 'not checked yet' }

  const shown = $derived.by(() => {
    if (!versions) return []
    const f = filter.trim().toLowerCase()
    const list = f ? versions.filter((v) => v.rel.toLowerCase().includes(f)) : versions
    return list.slice(0, 500)
  })

  const allOk = $derived(checks !== null && checks.length > 0 && checks.every((c) => c.ok))
  const typeLabel = (t: string) =>
    ({ sendonly: `this computer → ${there}`, receiveonly: `${there} → this computer`, sendreceive: 'both ways' })[t] ?? t
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-label="Conflicts, old versions, duplicates and checks">
  <div class="tabs" role="tablist" tabindex="-1">
    <button role="tab" aria-selected={tab === 'conflicts'} class:on={tab === 'conflicts'} onclick={() => show('conflicts')}>
      Conflicts{#if conflicts?.length}<span class="count">{conflicts.length}</span>{/if}
    </button>
    <button role="tab" aria-selected={tab === 'versions'} class:on={tab === 'versions'} onclick={() => show('versions')}>Old versions</button>
    <button role="tab" aria-selected={tab === 'check'} class:on={tab === 'check'} onclick={() => show('check')}>Is it all copied?</button>
    <button role="tab" aria-selected={tab === 'same'} class:on={tab === 'same'} onclick={() => show('same')}>Same everywhere?</button>
    <button role="tab" aria-selected={tab === 'duplicates'} class:on={tab === 'duplicates'} onclick={() => show('duplicates')}>Duplicates</button>
    <div class="grow"></div>
    <button class="close" onclick={onclose} aria-label="Close">✕</button>
  </div>

  <div class="body">
    {#if error}<p class="box bad">{error}</p>{/if}

    {#if tab === 'conflicts'}
      <p class="muted lead">
        A conflict happens when the same file was edited on both computers before they could catch up. Both versions are
        kept: the newer one under the normal name, the other with <span class="mono">sync-conflict</span> in its name. Pick
        the one to keep. The other one moves to Old versions, so nothing is lost.
      </p>
      {#if conflicts === null}
        <p class="muted">Looking…</p>
      {:else if conflicts.length === 0}
        <p class="empty">No conflicts. Nothing was edited on both computers at once.</p>
      {:else}
        <ul class="list">
          {#each conflicts as c (c.copy)}
            <li class="conflict">
              <div class="name">
                <b>{nameOf(c.original)}</b>
                <span class="mono muted small">{dirName(c.original) || '(top level)'}</span>
              </div>
              <div class="pair">
                <div class="side">
                  <span class="label">Current version</span>
                  {#if c.originalSize !== null}
                    <span>{bytes(c.originalSize)} · {when(c.originalMtime)}</span>
                    <button class="btn" disabled={working !== null} onclick={() => keep(c, false)}>Keep this</button>
                  {:else}
                    <span class="muted">deleted</span>
                  {/if}
                </div>
                <div class="side">
                  <span class="label">Other version</span>
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
        When a synced file is changed or deleted, its old copy is kept here for 30 days. <b>Put back</b> brings an old copy
        back, and the version it replaces is kept here too. In a folder that only receives from {there}, the old copy is
        saved next to the file instead, so {there} does not overwrite it.
      </p>
      <input type="search" placeholder="Search by file name or folder" bind:value={filter} />
      {#if versions === null}
        <p class="muted">Looking…</p>
      {:else if versions.length === 0}
        <p class="empty">Nothing kept yet. Old copies appear here once synced files are changed or deleted.</p>
      {:else}
        <ul class="list">
          {#each shown as k (k.folder + k.path + k.versionTime)}
            <li class="row">
              <div class="name">
                <b>{nameOf(k.rel)}</b>
                <span class="mono muted small">{dirName(k.rel) || '(top level)'}</span>
              </div>
              <span class="small muted nowrap" title="This copy was last edited {whenIso(k.modTime)}">replaced {whenIso(k.versionTime)}</span>
              <span class="small nowrap">{bytes(k.size)}</span>
              <button class="btn" disabled={working !== null} onclick={() => restore(k)}>
                {working === k.folder + k.path + k.versionTime ? 'Putting back…' : 'Put back'}
              </button>
            </li>
          {/each}
        </ul>
        {#if shown.length < (filter ? versions.filter((v) => v.rel.toLowerCase().includes(filter.trim().toLowerCase())).length : versions.length)}
          <p class="muted small">Showing the newest 500. Search to find older ones.</p>
        {/if}
      {/if}
    {:else if tab === 'same'}
      <p class="muted lead">
        Every few hours Setu reads every synced file on each computer and works out a fingerprint (SHA-256) of each top
        folder. When the fingerprints match, the folder holds exactly the same files, byte for byte, on both computers. When
        they differ, press the folder to see which files.
      </p>
      {#if same === null}
        <p class="muted">Looking…</p>
      {:else if same.length === 0}
        <p class="empty">Not checked yet. Press Check now; on a big folder the first check takes a few minutes.</p>
      {:else}
        <ul class="list">
          {#each same as r (r.top)}
            <li class="check">
              <span class="mark" class:ok={r.others.every((o) => o[2] === 'same')}>{r.others.every((o) => o[2] === 'same') ? '✓' : '!'}</span>
              <div class="name">
                <b>{r.top || 'Files at the top level'}</b>
                <span class="small muted">{r.files.toLocaleString()} files · {bytes(r.bytes)} · checked {when(r.checkedAt)}</span>
                {#each r.others as [id, name, word] (id)}
                  {#if word === 'differs'}
                    <button class="link-btn small" onclick={() => showDiff(id, name, r.top)}>{name}: different. Show which files</button>
                  {:else}
                    <span class="small {word === 'same' ? 'ok-text' : 'muted'}">{name}: {sameWord[word]}</span>
                  {/if}
                {/each}
                {#if diff && diff.top === r.top}
                  {#if diff.list === null}
                    <span class="small muted">{diff.name}'s file fingerprints haven't arrived yet. Press Check now on both computers, then look again.</span>
                  {:else}
                    {#each diff.list.slice(0, 200) as m (m.path)}<span class="small mono">{m.path}: {m.what}</span>{/each}
                    {#if diff.list.length > 200}<span class="small muted">and {diff.list.length - 200} more</span>{/if}
                  {/if}
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if tab === 'duplicates'}
      <p class="muted lead">
        Syncing can leave extra copies behind: a <span class="mono">sync-conflict</span> copy, or an old version, that is
        exactly the same as the file it belongs to. Setu removes those, but only after checking three ways that they are
        the same: same size, same fingerprint (SHA-256), and every byte compared. Your own files are never touched, even
        if two of them look alike. Every removal is listed here and can be put back.
      </p>
      {#if removed === null}
        <p class="muted">Looking…</p>
      {:else if removed.length === 0}
        <p class="empty">Nothing removed yet.</p>
      {:else}
        <ul class="list">
          {#each removed.slice(0, 500) as r (r.removed + r.at)}
            <li class="row">
              <div class="name">
                <b>{nameOf(r.removed)}</b>
                <span class="mono muted small">same as {r.kept}</span>
              </div>
              <span class="small muted nowrap">{r.kind} · {when(r.at)}</span>
              <span class="small nowrap">{bytes(r.size)}</span>
              {#if r.undone}
                <span class="small muted nowrap">put back</span>
              {:else}
                <button class="btn" disabled={working !== null} onclick={() => undoTidy(r)}>{working === r.removed ? 'Putting back…' : 'Put back'}</button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    {:else}
      <p class="muted lead">
        Syncthing checks every file piece by piece, so "all copied" means the files are truly identical, not only the same
        name and size. A folder gets a tick when this computer has nothing left to copy and no errors. The percentage shows
        how far {there} is.
      </p>
      {#if checks === null}
        <p class="muted">Checking…</p>
      {:else if checks.length === 0}
        <p class="empty">Nothing syncs yet. Use Quick setup or choose folders in the list, then press Start syncing.</p>
      {:else}
        {#if allOk}<p class="box okbox">All copied. All {checks.length} synced folders are complete on this computer.</p>{/if}
        <ul class="list">
          {#each checks as c (c.id)}
            <li class="check">
              <span class="mark" class:ok={c.ok}>{c.ok ? '✓' : '!'}</span>
              <div class="name">
                <b>{c.rel || 'Everything (top level)'}</b>
                <span class="small muted">
                  {typeLabel(c.folderType)} · {stateWord(c.state)}
                  {#if c.needFiles}· {c.needFiles.toLocaleString()} files ({bytes(c.needBytes)}) still to copy here{/if}
                  {#if c.otherCompletion !== null}· {there} {Math.floor(c.otherCompletion)}% done{/if}
                </span>
                {#if c.localChanges}
                  <span class="small warn">{c.localChanges} files edited here are not copied back, because this folder only receives from {there}.</span>
                {/if}
                {#each c.errors.slice(0, 5) as e, i (i)}<span class="small bad-text mono">{e}</span>{/each}
                {#each c.samples.slice(0, 5) as p (p)}<span class="small muted mono">still to copy: {p}</span>{/each}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>

  <footer>
    {#if tab === 'same'}
      <button class="btn" disabled={asked} onclick={checkNow} title="Compare every file on all computers now">{asked ? 'Checking… (keeps going if you close this)' : 'Check now'}</button>
    {:else if tab === 'duplicates'}
      <button class="btn" disabled={working !== null} onclick={tidyNow}>{working === 'tidy' ? 'Looking…' : 'Remove duplicates now'}</button>
    {:else if tab === 'check'}
      <button class="btn" disabled={loading} onclick={() => load('check', true)} title="Look through every folder for changes first, then check">
        {loading ? 'Checking…' : 'Look again and check'}
      </button>
    {:else}
      <button class="btn" disabled={loading} onclick={() => load(tab)}>{loading ? 'Looking…' : 'Refresh'}</button>
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
  .link-btn {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }
  .ok-text {
    color: var(--ok);
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
