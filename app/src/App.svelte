<script lang="ts">
  import { onMount } from 'svelte'
  import { SvelteMap } from 'svelte/reactivity'
  import { listen } from '@tauri-apps/api/event'
  import { api, type EngineStatus, type Update, type Flow, type FolderCheck, type NodeView, type PlanView, type Report, type Snapshot, type View } from './lib/api'
  import DirPicker from './lib/DirPicker.svelte'
  import FirstSync from './lib/FirstSync.svelte'
  import LeaveOut from './lib/LeaveOut.svelte'
  import PlanSheet from './lib/PlanSheet.svelte'
  import ProgressSheet, { pauseChoices, type PauseHow } from './lib/ProgressSheet.svelte'
  import TransferBar from './lib/TransferBar.svelte'
  import TrustSheet, { type TrustTab } from './lib/TrustSheet.svelte'
  import Tree, { type Row } from './lib/Tree.svelte'
  import { ago, bytes, called, clock, deviceState, dirOf, duration, modeOf, rate, sentence, shortLabel, type Dir } from './lib/format'

  let view = $state<View | null>(null)
  let rows = $state<Row[]>([])
  let selected = $state<string | null>(null)
  let plan = $state<PlanView | null>(null)
  let busy = $state(false)
  let error = $state<string | null>(null)
  let toast = $state<string | null>(null)
  let rootInput = $state('')
  let server = $state('')
  let email = $state('')
  let password = $state('')
  let recoveryKey = $state('')
  let needsRecovery = $state(false)
  /** How this computer takes part: the same files as the others, or a backup that only receives. */
  let role = $state<'same' | 'backup'>('same')
  let guide = $state(false)
  let trust = $state<TrustTab | null>(null)
  let checks = $state<FolderCheck[] | null>(null)
  let pauseMenu = $state(false)
  let moreMenu = $state(false)
  /** Copy progress, sent by Setu every few seconds. */
  let snap = $state<Snapshot | null>(null)
  let showProgress = $state(false)
  /** Speed limits here in KB/s, 0 = none: [download, upload]. */
  let limits = $state<[number, number] | null>(null)
  /** The sync engine inside Setu. Until it runs, the window explains what it waits for. */
  let engine = $state<EngineStatus>({ state: 'starting' })
  /** Old start-up pieces that could not be removed: dismissed. */
  let leftSeen = $state(false)
  let autoStart = $state<boolean | null>(null)
  /** This Setu's version, and a newer release when there is one. */
  let version = $state('')
  let update = $state<Update | null>(null)
  let updateLater = $state(false)
  /** 'idle', 'checking', 'none' (up to date), 'installing'. */
  let updateStep = $state<'idle' | 'checking' | 'none' | 'installing'>('idle')

  const me = $derived(view?.me ?? null)
  const other = $derived(view?.other ?? null)
  /** The other computer in everyday words ("Windows"). */
  const there = $derived(me && other ? called(other, me) : 'the other computer')
  const sel = $derived(rows.find((r) => r.node.path === selected)?.node ?? null)

  /** 'folder': one folder at a time, like File Explorer. 'tree': every level in one expanding list. */
  let layout = $state<'folder' | 'tree'>('folder')
  /** The folder shown in folder layout ('' = the whole synced folder), and where Back goes. */
  let cwd = $state('')
  let backStack = $state<string[]>([])
  /** Show only names containing this. */
  let filter = $state('')
  /** Ticked items, for one choice on many at once. */
  const ticked = new SvelteMap<string, NodeView>()
  const shown = $derived.by(() => {
    const f = filter.trim().toLowerCase()
    return f ? rows.filter((r) => r.node.name.toLowerCase().includes(f)) : rows
  })
  const crumbs = $derived(cwd ? cwd.split('/').map((name, i, all) => ({ name, path: all.slice(0, i + 1).join('/') })) : [])
  const tickedNodes = $derived([...ticked.values()])

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

  /** Re-read what is shown, keeping what is open, selected and ticked. */
  async function reloadOpen() {
    if (layout === 'folder') {
      try {
        rows = (await api.children(cwd)).map((node) => ({ node, depth: 0, open: false, loading: false }))
      } catch {
        // The folder went away: show the whole synced folder again.
        cwd = ''
        rows = (await api.children('')).map((node) => ({ node, depth: 0, open: false, loading: false }))
      }
    } else rows = await readTree()
    // Ticked items show their new choices.
    for (const r of rows) if (ticked.has(r.node.path)) ticked.set(r.node.path, r.node)
  }

  async function readTree(): Promise<Row[]> {
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
    return build('', 0)
  }

  /** Go into a folder (folder layout). `remember`: Back returns here. */
  async function go(path: string, remember = true) {
    if (path === cwd) return
    if (remember) backStack.push(cwd)
    cwd = path
    filter = ''
    selected = null
    ticked.clear()
    await reloadOpen().catch(fail)
  }

  function goBack() {
    const p = backStack.pop()
    if (p !== undefined) go(p, false)
  }

  function goUp() {
    if (cwd) go(cwd.includes('/') ? cwd.slice(0, cwd.lastIndexOf('/')) : '')
  }

  async function setLayout(l: 'folder' | 'tree') {
    layout = l
    filter = ''
    ticked.clear()
    await reloadOpen().catch(fail)
  }

  function tick(paths: string[], on: boolean) {
    for (const p of paths) {
      const n = rows.find((r) => r.node.path === p)?.node
      if (on && n) ticked.set(p, n)
      else ticked.delete(p)
    }
  }

  /** Open the shown folder in File Explorer (or the Linux file manager). */
  function openHere() {
    if (!view?.root) return
    const sep = view.root.includes('\\') ? '\\' : '/'
    api.openPath(cwd && layout === 'folder' ? view.root.replace(/[\\/]+$/, '') + sep + cwd.split('/').join(sep) : null).catch(fail)
  }

  /** The choices every ticked item can take. */
  const bulkAllowed = $derived(tickedNodes.reduce<Dir[]>((acc, n) => acc.filter((d) => allowed(n).includes(d)), ['off', 'out', 'both', 'in']))
  /** The ticked items' shared choice, or null when they differ. */
  const bulkValue = $derived.by((): Dir | null => {
    const ds = new Set(tickedNodes.map((n) => dirOf(n.effective, me?.id)))
    return ds.size === 1 ? [...ds][0] : null
  })

  async function pickMany(d: Dir | null) {
    if (!me || !other || !tickedNodes.length) return
    const n = tickedNodes.length
    try {
      view = await api.setRules(
        tickedNodes.map((x) => [x.path, x.dir]),
        d === null ? null : modeOf(d, me, other),
      )
      await reloadOpen()
      say(d === null ? `${n} item${n === 1 ? '' : 's'} now follow their folder.` : `${shortLabel(d, there)} for ${n} item${n === 1 ? '' : 's'}.`)
    } catch (e) {
      fail(e)
    }
  }

  async function boot(v: View) {
    view = v
    // Until the engine runs, the engine card says why; its "not running" errors add nothing.
    error = engine.state === 'running' ? v.message : null
    if (v.stage === 'ready') {
      await reloadOpen()
      if (!limits) api.limits().then((l) => (limits = l), () => {})
    }
  }

  /** Copy progress, read from Syncthing every few seconds. */
  async function poll() {
    if (view?.stage !== 'ready' || document.hidden) return
    try {
      checks = await api.verify(false)
    } catch {
      checks = null
    }
  }

  function onEngine(e: EngineStatus) {
    const was = engine.state
    engine = e
    if (e.state === 'running' && was !== 'running') api.refresh().then(boot).catch(fail)
  }

  onMount(() => {
    api.engine().then(onEngine, fail)
    api.state().then(boot).catch(fail)
    api.autostart().then((on) => (autoStart = on), () => {})
    api.version().then((v) => (version = v), () => {})
    const timer = setInterval(poll, 8000)
    setTimeout(poll, 1500)
    api.progress().then((s) => (snap = s ?? snap), () => {})
    // The tray menu opens a sheet directly.
    const offs = [
      listen<TrustTab>('open-trust', (e) => {
        plan = null
        guide = false
        showProgress = false
        trust = e.payload
      }),
      listen('open-progress', () => {
        plan = null
        guide = false
        trust = null
        showProgress = true
      }),
      listen<Snapshot>('progress', (e) => (snap = e.payload)),
      listen<EngineStatus>('engine-changed', (e) => onEngine(e.payload)),
      listen<Update>('update-available', (e) => {
        update = e.payload
        updateLater = false
      }),
      // Paused or resumed from the tray.
      listen('view-changed', () => {
        api.refresh().then((v) => (view = v), fail)
      }),
    ]
    return () => {
      clearInterval(timer)
      for (const off of offs) off.then((f) => f())
    }
  })

  async function toggle(n: NodeView) {
    if (layout === 'folder') return go(n.path)
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
    return dirOf(n.inherited, me?.id)
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
      say(
        r.added + r.updated + r.removed
          ? `Saved. ${there} picks up the new choices by itself within a minute or two.`
          : 'Saved. Nothing needed changing on this computer.',
      )
      poll()
    } catch (e) {
      fail(e)
    } finally {
      busy = false
    }
  }

  async function discard() {
    try {
      await boot(await api.discard())
      say('Changes undone.')
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

  async function browse() {
    try {
      const picked = await api.pickFolder()
      if (picked) rootInput = picked
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

  async function signIn() {
    busy = true
    error = null
    try {
      const v = await api.signIn(server, email, password)
      password = ''
      await boot(v)
    } catch (e) {
      if (String(e) === 'recovery-key-needed') {
        password = ''
        needsRecovery = true
      } else fail(e)
    } finally {
      busy = false
    }
  }

  async function unlock() {
    busy = true
    try {
      await boot(await api.recovery(recoveryKey))
      recoveryKey = ''
      needsRecovery = false
    } catch (e) {
      fail(e)
    } finally {
      busy = false
    }
  }

  async function signOut() {
    try {
      await boot(await api.signOut())
      say('Signed out. Folders already set up keep syncing; new choices stop arriving here.')
    } catch (e) {
      fail(e)
    }
  }

  async function join() {
    busy = true
    try {
      await boot(await api.join(role === 'backup'))
      say(role === 'backup' ? 'This computer is now a backup copy.' : 'This computer is in. Choose what to sync, or wait for the choices from your other computers.')
    } catch (e) {
      fail(e)
    } finally {
      busy = false
    }
  }

  /** Changes not saved yet: choices plus the "always left out" list. */
  const changes = $derived(view ? view.edits.length + view.leaveOut.added.length + view.leaveOut.removed.length : 0)

  async function setLeaveOut(list: string[]): Promise<boolean> {
    try {
      view = await api.setLeaveOut(list)
      await reloadOpen()
      return true
    } catch (e) {
      fail(e)
      return false
    }
  }

  /** Pause for some minutes, until tomorrow 7:00, until resumed (0), or resume (null). */
  async function pause(how: PauseHow | null) {
    pauseMenu = false
    const now = Math.floor(Date.now() / 1000)
    let until: number | null = null
    if (how === 'm15') until = now + 15 * 60
    else if (how === 'hour') until = now + 3600
    else if (how === 'h3') until = now + 3 * 3600
    else if (how === 'manual') until = 0
    else if (how === 'tomorrow') {
      const t = new Date()
      t.setDate(t.getDate() + 1)
      t.setHours(7, 0, 0, 0)
      until = Math.floor(t.getTime() / 1000)
    }
    try {
      view = await api.setPause(until)
      say(until === null ? 'Syncing resumed on this computer.' : 'Syncing paused on this computer.')
      poll()
    } catch (e) {
      fail(e)
    }
  }

  const pausedText = $derived.by(() => {
    const u = view?.pausedUntil
    if (u === null || u === undefined) return null
    if (u === 0) return 'until you resume it'
    return `until ${clock(u)}`
  })

  async function setAutoStart(on: boolean) {
    try {
      autoStart = await api.setAutostart(on)
    } catch (e) {
      fail(e)
    }
  }

  async function checkUpdate() {
    updateStep = 'checking'
    try {
      update = await api.checkUpdate()
      updateStep = update ? 'idle' : 'none'
      updateLater = false
    } catch (e) {
      updateStep = 'idle'
      fail(e)
    }
  }

  async function installUpdate() {
    if (!update) return
    updateStep = 'installing'
    try {
      await api.installUpdate(update.version)
      say(`Installing Setu ${update.version}. Setu closes for a moment and starts again.`)
    } catch (e) {
      updateStep = 'idle'
      fail(e)
    }
  }

  async function moveEngine() {
    busy = true
    try {
      await api.moveEngine()
    } catch (e) {
      fail(e)
    } finally {
      busy = false
    }
  }

  async function setQuiet(quiet: boolean) {
    try {
      view = await api.setQuietProgress(quiet)
    } catch (e) {
      fail(e)
    }
  }

  async function setLimit(which: 0 | 1, value: number) {
    if (!limits || !Number.isFinite(value) || value < 0) return
    const next: [number, number] = [...limits]
    next[which] = Math.round(value)
    try {
      limits = await api.setLimits(next[0], next[1])
      say(next[which] ? `${which ? 'Upload' : 'Download'} limited to ${next[which]} KB/s.` : `No ${which ? 'upload' : 'download'} limit.`)
    } catch (e) {
      fail(e)
    }
  }

  /** Items of the More menu: things used now and then. */
  async function more(what: 'versions' | 'same' | 'check' | 'duplicates' | 'rescan' | 'open' | 'refresh') {
    moreMenu = false
    if (what === 'refresh') return refresh()
    if (what === 'open') return api.openPath(null).catch(fail)
    if (what === 'rescan') {
      try {
        await api.verify(true)
        say('Looking through every synced folder for changes.')
      } catch (e) {
        fail(e)
      }
      return
    }
    trust = what
  }

  /** Close an open menu when clicking anywhere outside it. */
  function outside(e: MouseEvent) {
    if ((e.target as Element | null)?.closest('.menuwrap')) return
    pauseMenu = false
    moreMenu = false
  }

  /** "45% of 2.1 GB · 12 MB/s · 3 min left" for one direction. */
  function flowText(f: Flow): string {
    const left = snap?.paused ? 'paused' : f.etaSecs !== null ? `${rate(f.speed)} · about ${duration(f.etaSecs)} left` : f.speed ? rate(f.speed) : 'starting'
    return `${Math.floor(f.percent)}% of ${bytes(f.totalBytes)} · ${left}`
  }

  /** The direction to show in the header and the bottom bar: the one with more left. */
  const lead = $derived.by((): Flow | null => {
    if (!snap) return null
    const fs = [snap.down, snap.up].filter((f) => f.active)
    return fs.sort((a, b) => b.leftBytes - a.leftBytes)[0] ?? null
  })

  /** Quick setup: choose for the whole folder. */
  async function pickRoot(d: Dir) {
    await pick('', true, d)
  }

  async function changed(message: string) {
    say(message)
    try {
      view = await api.refresh()
      await reloadOpen()
    } catch (e) {
      fail(e)
    }
  }

  function onkey(e: KeyboardEvent) {
    const typing = (e.target as Element | null)?.closest('input, textarea')
    if (e.key === 'Escape' && (pauseMenu || moreMenu)) pauseMenu = moreMenu = false
    else if (layout === 'folder' && e.altKey && e.key === 'ArrowLeft' && !plan) {
      e.preventDefault()
      goBack()
    } else if (layout === 'folder' && e.altKey && e.key === 'ArrowUp' && !plan) {
      e.preventDefault()
      goUp()
    } else if (layout === 'folder' && e.key === 'Backspace' && !typing && !plan && !trust && !showProgress) {
      e.preventDefault()
      goUp()
    } else if (e.key === 'Escape' && ticked.size && !plan && !trust && !showProgress) ticked.clear()
    else if (e.key === 'Escape' && showProgress) showProgress = false
    else if (e.key === 'Escape' && trust) trust = null
    else if (e.key === 'Escape' && plan) plan = null
    else if ((e.key === 'F5' || (e.ctrlKey && e.key === 'r')) && !plan) {
      e.preventDefault()
      refresh()
    }
  }

  const revState = $derived.by(() => {
    if (!view || view.sharedRevision === null) return null
    const want = view.sharedRevision
    const side = (r: number | null) => (r === want ? 'ok' : r === null ? 'none' : 'behind')
    return { want, me: side(me?.appliedRevision ?? null), other: side(other?.appliedRevision ?? null), otherHeld: !!other?.held }
  })

  /** One plain line about copying, for the bottom bar. */
  const progress = $derived.by((): { text: string; tone: 'ok' | 'busy' | 'wait' | 'bad'; pct?: number } | null => {
    if (pausedText) return { text: `Paused by you ${pausedText}. Nothing is copied to or from this computer.`, tone: 'wait' }
    if (snap && other && (snap.down.active || snap.up.active) && !snap.paused && other.connected) {
      const parts = []
      if (snap.down.active) parts.push(`To this computer: ${flowText(snap.down)}`)
      if (snap.up.active) parts.push(`To ${there}: ${flowText(snap.up)}`)
      return { text: parts.join('   ·   '), tone: 'busy', pct: lead?.percent }
    }
    if (!checks || !other) return null
    if (checks.length === 0) return { text: 'Nothing syncs yet. Press Quick setup to choose what to copy.', tone: 'wait' }
    if (checks.some((c) => c.errors.length)) return { text: 'Some files could not be copied. Press to see which.', tone: 'bad' }
    if (revState && revState.me !== 'ok') return null
    if (revState?.otherHeld) return { text: `${there} needs you: the new choices would replace or remove many files there, so open Setu on ${there} to check them.`, tone: 'wait' }
    if (revState && revState.other !== 'ok') return { text: `${there} is setting up the new choices. It does this by itself when it is on.`, tone: 'busy' }
    if (!other.connected) return { text: `${there} is offline. Copying continues when both computers are on and connected.`, tone: 'wait' }
    const needFiles = checks.reduce((s, c) => s + c.needFiles, 0)
    const needBytes = checks.reduce((s, c) => s + c.needBytes, 0)
    if (needFiles) return { text: `Copying to this computer: ${needFiles.toLocaleString()} files (${bytes(needBytes)}) left.`, tone: 'busy' }
    const behind = checks.filter((c) => c.otherCompletion !== null && c.otherCompletion < 100)
    if (behind.length) {
      const pct = Math.floor(Math.min(...behind.map((c) => c.otherCompletion ?? 100)))
      return { text: `Copying to ${there}: ${pct}% done.`, tone: 'busy' }
    }
    if (checks.some((c) => c.state !== 'idle')) return { text: 'Looking for changes…', tone: 'busy' }
    return { text: `✓ All copied. Both computers have the same files in ${checks.length === 1 ? 'the synced folder' : `all ${checks.length} synced folders`}.`, tone: 'ok' }
  })
</script>

<svelte:window onkeydown={onkey} onclick={outside} />

<div class="shell">
  <header class="top">
    <div class="brand">
      <img src="/favicon.svg" alt="" width="26" height="26" />
      <span class="word">Setu</span>
    </div>
    {#if view?.stage === 'ready' && me && other}
      <div class="link">
        <span class="dev" title={me.name}>This computer<small>{me.name}</small></span>
        <span class="wire" class:up={other.connected} title={other.connected ? `Connected${other.address ? ' via ' + other.address : ''}` : 'Not connected'}>
          <span class="dot"></span>
        </span>
        <span class="dev" title={other.name}>{there}<small>{other.connected ? 'connected' : 'offline'}</small></span>
      </div>
      {#if revState}
        {@const o = deviceState(other, revState.want)}
        <span
          class="rev {revState.me === 'ok' && o.tone === 'ok' ? 'ok' : 'behind'}"
          title="Sync choices version {revState.want}. This computer has {me.appliedRevision ?? 'none'}, {there} has {other.appliedRevision ?? 'none'}."
        >
          {revState.me !== 'ok' ? (me.held ? 'New choices need you' : 'Setting up here') : `${there}: ${o.text}`}
        </span>
      {/if}
      {#each view.devices.filter((d) => d.inRules) as d (d.id)}
        {@const o = deviceState(d, revState?.want ?? null)}
        <span class="rev {o.tone === 'ok' ? 'ok' : 'behind'}" title="{d.name}{d.receiveOnly ? ' (backup copy)' : ''}">{d.name}: {o.text}</span>
      {/each}
    {/if}
    <div class="grow"></div>
    {#if view?.stage === 'ready'}
      {#if view.fileConflicts}
        <button class="btn conflicts" onclick={() => (trust = 'conflicts')} title="Files edited on both computers: pick which version to keep">
          {view.fileConflicts} conflict{view.fileConflicts === 1 ? '' : 's'}
        </button>
      {/if}
      <button class="btn prog" class:going={!!lead} onclick={() => (showProgress = true)} title="Copy progress: speed, time so far, time left, per folder">
        {#if lead}<span class="spin" aria-hidden="true"></span>{Math.floor(lead.percent)}%{:else}Progress{/if}
      </button>
      <div class="menuwrap">
        {#if pausedText}
          <button class="btn primary" onclick={() => pause(null)} title="Paused {pausedText}">Resume</button>
        {:else}
          <button class="btn" onclick={() => ((pauseMenu = !pauseMenu), (moreMenu = false))} aria-expanded={pauseMenu} title="Stop copying on this computer for a while">Pause</button>
          {#if pauseMenu}
            <div class="menu" role="menu">
              {#each pauseChoices as c (c.how)}
                <button role="menuitem" onclick={() => pause(c.how)}>{c.label}</button>
              {/each}
            </div>
          {/if}
        {/if}
      </div>
      <button class="btn hide-narrow" onclick={() => (guide = true)} title="Choose what to copy in two steps">Quick setup</button>
      <div class="menuwrap">
        <button class="btn" onclick={() => ((moreMenu = !moreMenu), (pauseMenu = false))} aria-expanded={moreMenu} aria-haspopup="menu" title="More">More ▾</button>
        {#if moreMenu}
          <div class="menu" role="menu">
            <button role="menuitem" class="show-narrow" onclick={() => ((moreMenu = false), (guide = true))}>Quick setup…</button>
            <button role="menuitem" onclick={() => more('versions')}>Old versions…</button>
            <button role="menuitem" onclick={() => more('check')}>Is it all copied?…</button>
            <button role="menuitem" onclick={() => more('same')}>Same everywhere?…</button>
            <button role="menuitem" onclick={() => more('duplicates')}>Duplicates removed…</button>
            <hr />
            <button role="menuitem" onclick={() => more('rescan')}>Look for changes now</button>
            <button role="menuitem" onclick={() => more('open')}>Open this computer's folder</button>
            <button role="menuitem" onclick={() => more('refresh')}>Refresh <span class="key">F5</span></button>
          </div>
        {/if}
      </div>
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
      Sync settings were changed on both computers at the same time. Setu kept one set; the other is saved as
      {view.conflicts.join(', ')}. Check your choices below, then press Review again.
    </div>
  {/if}

  {#each view?.watch?.holds ?? [] as h (h.folder)}
    <div class="banner bad" role="alert">
      <span>
        <b>Paused for safety: {h.rel === null ? h.folder : h.rel || 'everything'}.</b> {h.deletes} files were deleted on
        {there} within two minutes, so Setu paused this folder here before the deletes reached it. If that was meant, press Resume. If not, nothing
        is lost yet: fix it on {there} first, and any file already removed can be got back under Old versions for 30 days.
      </span>
      <button class="btn" onclick={() => resume(h.folder)}>Resume syncing</button>
    </div>
  {/each}
  {#if pausedText}
    <div class="banner warn">
      <span>Syncing is paused on this computer {pausedText}. Files stay as they are; changes on either side wait and are copied after you resume.</span>
      <button class="btn" onclick={() => pause(null)}>Resume now</button>
    </div>
  {/if}
  {#if view?.stage === 'ready' && view.watch && view.watch.running && !view.watch.gateOpen && view.watch.reason !== 'you paused syncing'}
    <div class="banner warn">
      <span>Copying is on hold: {view.watch.reason?.replace('you paused syncing, ', '')}. It starts again by itself when the connection is suitable.</span>
      {#if view.watch.metered && !view.watch.allowMetered}<button class="btn" onclick={() => allow('metered', true)}>Copy anyway on this network</button>{/if}
      {#if view.watch.relayed && !view.watch.allowRelay}<button class="btn" onclick={() => allow('relay', true)}>Copy anyway through the relay</button>{/if}
    </div>
  {/if}
  {#if engine.state === 'running' && view?.stage === 'ready' && view.watch && !view.watch.running}
    <div class="banner warn">
      <span>Setu's safety checks are not running yet: pausing when many files are deleted at once, and waiting for a suitable network. Files still copy. They start within a minute; if this stays, quit Setu from the tray and start it again.</span>
    </div>
  {/if}
  {#if engine.state === 'running' && engine.left.length && !leftSeen}
    <div class="banner warn">
      <span>Syncthing now runs inside Setu. Some pieces of the old setup could not be removed; remove them yourself when convenient: {engine.left.join(', ')}.</span>
      <button onclick={() => (leftSeen = true)} aria-label="Dismiss">✕</button>
    </div>
  {/if}

  {#if update && !updateLater}
    <div class="banner warn">
      <span>
        {#if updateStep === 'installing'}Downloading and installing Setu {update.version}… Setu closes for a moment and starts again by itself.
        {:else}<b>Setu {update.version} is available</b> (you have {version}). Your rules, settings and files stay as they are.{/if}
      </span>
      {#if updateStep !== 'installing'}
        <button class="btn primary" onclick={installUpdate}>Update now</button>
        <button class="btn" onclick={() => (updateLater = true)}>Later</button>
      {/if}
    </div>
  {/if}

  {#if engine.state !== 'running'}
    <div class="center">
      <div class="card">
        {#if engine.state === 'move'}
          <h1>Move Syncthing into Setu</h1>
          <p class="muted">
            Setu now has its sync engine built in, so nothing else needs to run in the background. Syncthing still runs on its own on this computer, so
            Setu takes it over once. This is what happens:
          </p>
          <ol class="steps">
            {#each engine.steps as s}<li>{s}</li>{/each}
          </ol>
          <div class="row-end"><button class="btn primary" disabled={busy} onclick={moveEngine}>Move it into Setu</button></div>
        {:else if engine.state === 'moving'}
          <h1>Moving Syncthing into Setu…</h1>
          <p class="muted">Stopping the old Syncthing, copying its identity, settings and file index, then checking the result. This takes about a minute.</p>
        {:else if engine.state === 'tailnet'}
          <h1>Waiting for Tailscale</h1>
          <p class="muted">
            Setu copies files only over Tailscale, your private network, and this computer has no Tailscale address yet. Start Tailscale and sign in;
            Setu goes on by itself.
          </p>
        {:else if engine.state === 'failed'}
          <h1>The sync engine could not start</h1>
          <p>{engine.message}</p>
          <p class="muted small">Details are in the engine's log, <span class="mono">engine.log</span>, in Setu's engine folder.</p>
        {:else}
          <h1>Starting…</h1>
          <p class="muted">Starting Setu's sync engine.</p>
        {/if}
      </div>
    </div>
  {:else if !view}
    <div class="center muted">Starting…</div>
  {:else if view.stage === 'setup'}
    <div class="center">
      <div class="card">
        <h1>Which folder should Setu look after?</h1>
        <p class="muted">Press Browse and pick the folder on this computer that holds the files you want to sync, or type its path. Nothing is copied until you choose what goes where and press Start syncing.</p>
        <div class="pickrow">
          <input type="text" placeholder="/home/you/Work or C:\Work" aria-label="Folder to sync" bind:value={rootInput} onkeydown={(e) => e.key === 'Enter' && init()} />
          <button class="btn" onclick={browse}>Browse…</button>
        </div>
        <div class="row-end"><button class="btn primary" disabled={!rootInput.trim()} onclick={init}>Continue</button></div>
      </div>
    </div>
  {:else if view.stage === 'signin'}
    <div class="center">
      <div class="card">
        {#if needsRecovery}
          <h1>Unlock with your recovery key</h1>
          <p class="muted">Your Brahma password was reset, so your data needs the recovery key you saved when you made the account. It is only used on this computer.</p>
          <input type="password" placeholder="Recovery key" autocomplete="off" bind:value={recoveryKey} onkeydown={(e) => e.key === 'Enter' && unlock()} />
          <div class="row-end">
            <button class="btn" onclick={() => (needsRecovery = false)}>Back</button>
            <button class="btn primary" disabled={busy || !recoveryKey.trim()} onclick={unlock}>Unlock</button>
          </div>
        {:else}
          <h1>Sign in with your Brahma account</h1>
          <p class="muted">
            Your computers find each other through your account, and your sync choices are shared through it, locked so only
            your computers can read them. The files themselves go straight from computer to computer over Tailscale, never
            through the server.
          </p>
          <label class="stack">Server address<input type="url" placeholder="https://your-server" autocomplete="url" bind:value={server} /></label>
          <label class="stack">Email<input type="email" autocomplete="username" bind:value={email} /></label>
          <label class="stack">Password<input type="password" autocomplete="current-password" bind:value={password} onkeydown={(e) => e.key === 'Enter' && signIn()} /></label>
          <div class="row-end"><button class="btn primary" disabled={busy || !server.trim() || !email.trim() || !password} onclick={signIn}>{busy ? 'Signing in…' : 'Sign in'}</button></div>
        {/if}
      </div>
    </div>
  {:else if view.stage === 'join'}
    <div class="center">
      <div class="card">
        <h1>How should this computer take part?</h1>
        <p class="muted">
          Signed in as {view.account?.email}.
          {#if view.devices.length > 1}
            Your account already has {view.devices.length - 1} other computer{view.devices.length === 2 ? '' : 's'}: {view.devices.slice(1).map((d) => d.name).join(', ')}.
            Their sync choices apply here too, and Setu shows you what would change before anything big happens.
          {:else}
            This is the first computer in your account. Nothing syncs until you choose what to copy.
          {/if}
        </p>
        <label class="choice" class:on={role === 'same'}>
          <input type="radio" bind:group={role} value="same" />
          <span><b>Keep the same files as my other computers</b><small>Changes made on any computer reach the others.</small></span>
        </label>
        <label class="choice" class:on={role === 'backup'}>
          <input type="radio" bind:group={role} value="backup" />
          <span><b>Backup copy</b><small>This computer only receives. Changes made here are never sent back, so it can't spoil the others.</small></span>
        </label>
        <p class="muted mono small">{view.root}</p>
        <div class="row-end"><button class="btn primary" disabled={busy} onclick={join}>{busy ? 'Adding…' : 'Continue'}</button></div>
      </div>
    </div>
  {:else if me && other}
    <main class="main">
      <section class="treepane">
        <button class="rootrow" class:sel={selected === null} onclick={() => (selected = null)}>
          <span class="mono path">{view.root}</span>
          <span class="muted small">{view.localEntries.toLocaleString()} items here{view.remoteEntries !== null ? ` · ${view.remoteEntries.toLocaleString()} on ${there}` : ''}</span>
          <span class="chip {dirOf(view.rootMode, me.id)}">{shortLabel(dirOf(view.rootMode, me.id), there)}</span>
        </button>
        {#if view.remoteEntries === null}
          <p class="hint">{there}'s file list hasn't arrived yet, so only this computer's files show. It arrives by itself while Setu runs on {there}.</p>
        {/if}
        <div class="navbar">
          {#if layout === 'folder'}
            <button class="iconbtn" onclick={goBack} disabled={!backStack.length} title="Back (Alt+←)" aria-label="Back">←</button>
            <button class="iconbtn" onclick={goUp} disabled={!cwd} title="Up one folder (Alt+↑)" aria-label="Up one folder">↑</button>
            <nav class="crumbs" aria-label="Folder path">
              <button class="bc" class:here={!cwd} onclick={() => go('')}>All files</button>
              {#each crumbs as c (c.path)}
                <span class="sep" aria-hidden="true">›</span>
                <button class="bc" class:here={c.path === cwd} onclick={() => go(c.path)} title={c.path}>{c.name}</button>
              {/each}
            </nav>
          {:else}
            <span class="crumbs muted small">Every folder, opened in place</span>
          {/if}
          <div class="grow"></div>
          <input class="filter" type="search" placeholder="Filter names…" aria-label="Filter names" bind:value={filter} />
          <div class="seg" role="radiogroup" aria-label="Layout">
            <button role="radio" aria-checked={layout === 'folder'} class:on={layout === 'folder'} onclick={() => setLayout('folder')} title="One folder at a time, like File Explorer">Folders</button>
            <button role="radio" aria-checked={layout === 'tree'} class:on={layout === 'tree'} onclick={() => setLayout('tree')} title="Every level in one expanding list">Tree</button>
          </div>
          <button class="btn" onclick={openHere} title="Open this folder in your file manager">Open in {me.os === 'windows' ? 'File Explorer' : 'Files'}</button>
        </div>
        <Tree
          rows={shown}
          {selected}
          checked={ticked}
          flat={layout === 'folder'}
          meId={me.id}
          {there}
          remoteKnown={view.remoteEntries !== null}
          onselect={(n) => (selected = n.path)}
          ontoggle={toggle}
          oncheck={tick}
        />
        {#if filter && !shown.length}<p class="hint">No names here contain “{filter}”.</p>{/if}
      </section>

      <aside class="details">
        {#if tickedNodes.length}
          {@const n = tickedNodes.length}
          {@const nd = tickedNodes.filter((x) => x.dir).length}
          {@const nf = n - nd}
          <h2 class="title">{n} ticked</h2>
          <p class="muted small">
            {nd} folder{nd === 1 ? '' : 's'} · {nf} file{nf === 1 ? '' : 's'}
            · <button class="link-btn" onclick={() => ticked.clear()}>Untick all</button>
          </p>
          <p class="say">Choose once for all {n}:</p>
          <DirPicker value={bulkValue} allowed={bulkAllowed} {there} onpick={pickMany} />
          {#if bulkAllowed.length < 4}
            <p class="muted small">Some ticked files are in a folder that syncs, so they can only follow it or be left out. To send them a different way, set their folder to Don't sync first.</p>
          {/if}
          <p class="muted small">
            <button class="link-btn" onclick={() => pickMany(null)}>Make all {n} follow their folder</button>
          </p>
        {:else if sel}
          {@const d = dirOf(sel.effective, me.id)}
          <h2 class="title" title={sel.path}>{sel.name}</h2>
          <p class="mono muted crumb">{sel.path}</p>
          <DirPicker value={d} allowed={allowed(sel)} {there} onpick={(x) => pick(sel.path, sel.dir, x)} />
          <p class="say">{sentence(d, there)}</p>
          <p class="muted small">
            {#if sel.explicit !== null}
              Chosen for this {sel.dir ? 'folder and everything in it' : 'file'}.
              <button class="link-btn" onclick={() => pick(sel.path, sel.dir, null)}>Follow the folder above instead</button>
            {:else}
              Follows {sel.from ? sel.from : 'the top-level choice'}.{sel.dir ? ' Choose here to treat this folder differently.' : ''}
            {/if}
          </p>
          {#if !sel.dir && parentDir(sel) !== 'off'}
            <p class="muted small">A single file can only be left out here. To send it a different way from its folder, set the folder to Don't sync first.</p>
          {/if}
          <dl class="facts">
            <dt>This computer</dt>
            <dd>{sel.here ? `${bytes(sel.here.size)}${sel.dir ? ` · ${sel.here.files.toLocaleString()} files` : ''}` : 'not here'}</dd>
            <dt>{there}</dt>
            <dd>{sel.there ? `${bytes(sel.there.size)}${sel.dir ? ` · ${sel.there.files.toLocaleString()} files` : ''}` : view.remoteEntries === null ? 'list not arrived yet' : 'not there'}</dd>
          </dl>
        {:else}
          {@const d = dirOf(view.rootMode, me.id)}
          <h2 class="title">Everything</h2>
          <p class="mono muted crumb">{view.root}</p>
          <DirPicker value={d} {there} onpick={(x) => pick('', true, x)} />
          <p class="say">{sentence(d, there)}</p>
          <p class="muted small">Every folder and file follows this, except the ones you choose differently in the list. To leave a big project out, click it in the list and choose Don't sync.</p>
          <LeaveOut list={view.alwaysIgnore} onchange={setLeaveOut} />
          <p class="muted small">{there}'s file list last updated: {ago(view.remoteListAge)}</p>
          <div class="never">
            <h3>Your computers</h3>
            {#each [me, other, ...view.devices] as d (d.id)}
              {@const o = deviceState(d, view.sharedRevision)}
              <p class="small">
                <b>{d.id === me.id ? 'This computer' : d.name}</b>{d.receiveOnly ? ' (backup copy)' : ''}: {o.text}{d.id !== me.id ? `, ${d.connected ? 'connected' : 'offline'}` : ''}
                {#if d.held}<br /><span class="muted">{d.held.reasons[0]}</span>{/if}
                {#each d.problems.slice(0, 3) as pr, i (i)}<br /><span class="muted">{pr}</span>{/each}
              </p>
            {/each}
            {#if view.account}
              <p class="muted small">Brahma account: {view.account.email} <button class="link-btn" onclick={signOut}>Sign out</button></p>
            {/if}
          </div>
          {#if view.watch}
            <div class="never">
              <h3>Network</h3>
              <label class="tog"><input type="checkbox" checked={view.watch.allowMetered} onchange={(e) => allow('metered', (e.currentTarget as HTMLInputElement).checked)} /> Copy on limited-data connections (phone hotspot)</label>
              <label class="tog"><input type="checkbox" checked={view.watch.allowRelay} onchange={(e) => allow('relay', (e.currentTarget as HTMLInputElement).checked)} /> Copy through an internet relay when a direct link fails (slower)</label>
              <p class="muted small">Now: {view.watch.running ? `${view.watch.metered ? 'limited-data connection' : 'normal connection'}, ${view.watch.relayed ? 'through a relay' : 'direct link'}` : 'helper not running'}</p>
            </div>
          {/if}
          <div class="never">
            <h3>Updates</h3>
            <p class="muted small">This is Setu {version}. Setu looks for a new version twice a day and tells you; it installs only when you say so.</p>
            {#if update}
              <p class="small"><b>Setu {update.version}</b> is available.</p>
              {#if update.notes}<pre class="notes">{update.notes}</pre>{/if}
              <button class="btn primary" disabled={updateStep === 'installing'} onclick={installUpdate}>{updateStep === 'installing' ? 'Installing…' : 'Update now'}</button>
            {:else}
              <button class="btn" disabled={updateStep === 'checking'} onclick={checkUpdate}>{updateStep === 'checking' ? 'Checking…' : 'Check for updates'}</button>
              {#if updateStep === 'none'}<span class="muted small">You have the newest version.</span>{/if}
            {/if}
          </div>
          {#if autoStart !== null}
            <div class="never">
              <h3>Start-up</h3>
              <label class="tog"><input type="checkbox" checked={autoStart} onchange={(e) => setAutoStart((e.currentTarget as HTMLInputElement).checked)} /> Start Setu when I sign in to this computer</label>
              <p class="muted small">Setu keeps your folders in sync only while it runs. Closing the window keeps it running; Quit Setu in the tray stops it.</p>
            </div>
          {/if}
          <div class="never">
            <h3>Speed and notifications</h3>
            <label class="tog"><input type="checkbox" checked={!view.quietProgress} onchange={(e) => setQuiet(!(e.currentTarget as HTMLInputElement).checked)} /> Show copy progress in desktop notifications</label>
            {#if limits}
              {@const lim = limits}
              <label class="tog lim">Download limit <input type="number" min="0" step="100" value={lim[0]} onchange={(e) => setLimit(0, (e.currentTarget as HTMLInputElement).valueAsNumber)} /> KB/s</label>
              <label class="tog lim">Upload limit <input type="number" min="0" step="100" value={lim[1]} onchange={(e) => setLimit(1, (e.currentTarget as HTMLInputElement).valueAsNumber)} /> KB/s</label>
              <p class="muted small">0 means no limit. Applies to all copying on this computer.</p>
            {/if}
          </div>
        {/if}
      </aside>
    </main>

    <footer class="bar" class:live={changes > 0}>
      {#if changes}
        <span><b>{changes}</b> change{changes === 1 ? '' : 's'} not saved yet</span>
        <div class="grow"></div>
        <button class="btn" onclick={discard}>Undo changes</button>
        <button class="btn primary" onclick={review}>Review</button>
      {:else if revState && revState.me !== 'ok' && me.held}
        <span><b>New sync choices need you.</b> {me.held.reasons[0] ?? ''} Review them, then press Start syncing.</span>
        <div class="grow"></div>
        <button class="btn primary" onclick={review}>Review</button>
      {:else if revState && revState.me !== 'ok'}
        <span>Setting up the new sync choices here. If it takes more than a couple of minutes, review them yourself.</span>
        <div class="grow"></div>
        <button class="btn primary" onclick={review}>Review</button>
      {:else if snap && other && other.connected && !pausedText && !snap.paused && (snap.down.active || snap.up.active)}
        <TransferBar {snap} {there} onopen={() => (showProgress = true)} />
      {:else if progress}
        <button class="status {progress.tone}" onclick={() => (progress.tone === 'bad' ? (trust = 'check') : (showProgress = true))} title={progress.text}>
          {#if progress.tone === 'busy'}<span class="spin" aria-hidden="true"></span>{/if}
          {#if progress.pct !== undefined}<span class="mini" aria-hidden="true"><span style="width:{progress.pct}%"></span></span>{/if}
          <span class="txt">{progress.text}</span>
        </button>
      {:else}
        <span class="muted">Checking progress…</span>
      {/if}
    </footer>
  {/if}

  {#if toast}<div class="toast" role="status">{toast}</div>{/if}
  {#if guide && me && other && view}
    <FirstSync
      folders={rows.filter((r) => r.depth === 0 && r.node.dir).map((r) => r.node)}
      {me}
      {there}
      remoteKnown={view.remoteEntries !== null}
      rootDir={dirOf(view.rootMode, me.id)}
      onroot={pickRoot}
      onpick={(n, d) => pick(n.path, true, d)}
      onclose={() => (guide = false)}
      onreview={review}
    />
  {/if}
  {#if plan && me && other}
    <PlanSheet view={plan} {me} {there} {busy} onclose={() => (plan = null)} onapply={apply} />
  {/if}
  {#if showProgress && me && other}
    <ProgressSheet {snap} {there} {pausedText} onpause={pause} onclose={() => (showProgress = false)} onchanged={say} />
  {/if}
  {#if trust && me && other}
    <TrustSheet bind:tab={trust} {there} onclose={() => (trust = null)} onchanged={changed} />
  {/if}
</div>

<style>
  .btn.conflicts {
    background: var(--warn-bg);
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 40%, transparent);
    font-weight: 600;
  }
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
  .rev {
    padding: 3px 10px;
    border-radius: 99px;
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }
  .rev.ok {
    color: var(--ok);
    background: color-mix(in srgb, var(--ok) 13%, transparent);
  }
  .rev.behind {
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
  .steps {
    margin: 0 0 16px;
    padding-left: 20px;
    max-height: 45vh;
    overflow-y: auto;
    font-size: 13px;
    line-height: 1.5;
  }
  .steps li + li {
    margin-top: 6px;
  }
  .notes {
    max-height: 160px;
    overflow: auto;
    white-space: pre-wrap;
    font: inherit;
    font-size: 12px;
    margin: 0 0 8px;
  }
  .stack {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 10px;
    font-size: 13px;
    font-weight: 600;
  }
  .choice {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 12px 14px;
    margin-bottom: 8px;
    border: 1px solid var(--line);
    border-radius: 10px;
    cursor: pointer;
  }
  .choice.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .choice small {
    display: block;
    color: var(--muted);
    margin-top: 2px;
  }
  .row-end .btn + .btn {
    margin-left: 8px;
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
  .top .btn {
    flex: none;
    white-space: nowrap;
  }
  .top .link {
    min-width: 0;
  }
  @media (max-width: 1100px) {
    .rev {
      display: none;
    }
  }
  @media (max-width: 860px) {
    .hide-narrow {
      display: none;
    }
    .show-narrow {
      display: block;
    }
    .top {
      gap: 10px;
    }
  }
  @media (max-width: 760px) {
    .link .dev:first-child,
    .brand .word {
      display: none;
    }
  }
  @media (max-width: 900px) {
    .main {
      grid-template-columns: minmax(0, 1fr) 260px;
    }
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
  .navbar {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 2px 0 8px;
  }
  .iconbtn {
    flex: none;
    width: 30px;
    height: 30px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: var(--surface);
    font-size: 15px;
    transition: transform 100ms ease-out;
  }
  .iconbtn:hover:not(:disabled) {
    background: var(--surface-2);
  }
  .iconbtn:active:not(:disabled) {
    transform: scale(0.95);
  }
  .iconbtn:disabled {
    opacity: 0.35;
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .bc {
    min-width: 0;
    max-width: 180px;
    padding: 4px 7px;
    border-radius: 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--muted);
    font-weight: 550;
  }
  .bc:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .bc.here {
    color: var(--text);
    font-weight: 650;
  }
  .sep {
    color: var(--faint);
  }
  .filter {
    flex: 0 1 170px;
    min-width: 90px;
    height: 30px;
  }
  .seg {
    flex: none;
    display: flex;
    padding: 2px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: var(--surface-2);
  }
  .seg button {
    padding: 3px 10px;
    border-radius: 6px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .seg button.on {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 1px 2px rgb(0 0 0 / 12%);
  }
  @media (max-width: 900px) {
    .navbar > .btn {
      display: none;
    }
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
    flex-wrap: wrap;
    gap: 8px;
    min-height: 56px;
    padding: 8px 16px;
    border-top: 1px solid var(--line);
    background: var(--surface);
  }
  .bar > span {
    min-width: 0;
  }
  .bar.live {
    background: color-mix(in srgb, var(--accent) 7%, var(--surface));
  }
  .menuwrap {
    position: relative;
    flex: none;
  }
  .menu hr {
    width: 100%;
    margin: 4px 0;
    border: 0;
    border-top: 1px solid var(--line);
  }
  .key {
    float: right;
    margin-left: 16px;
    color: var(--faint);
    font-size: 12px;
  }
  .prog.going {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-width: 76px;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }
  .mini {
    flex: none;
    width: 70px;
    height: 5px;
    border-radius: 5px;
    background: color-mix(in srgb, currentColor 18%, transparent);
    overflow: hidden;
  }
  .mini span {
    display: block;
    height: 100%;
    background: currentColor;
    transition: width 600ms ease;
  }
  .status .txt {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pickrow {
    display: flex;
    gap: 8px;
  }
  .pickrow input {
    flex: 1;
    min-width: 0;
  }
  .lim input {
    width: 90px;
    margin-left: auto;
  }
  .show-narrow {
    display: none;
  }
  .menu {
    position: absolute;
    top: calc(100% + 6px);
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
  .menu button {
    padding: 8px 10px;
    border-radius: 7px;
    text-align: left;
    white-space: nowrap;
  }
  .menu button:hover {
    background: var(--surface-2);
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    max-width: 100%;
    padding: 6px 12px;
    border-radius: 99px;
    font-weight: 600;
    font-size: 13px;
    text-align: left;
  }
  .status.ok {
    color: var(--ok);
    background: color-mix(in srgb, var(--ok) 12%, transparent);
  }
  .status.busy {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .status.wait {
    color: var(--warn);
    background: var(--warn-bg);
  }
  .status.bad {
    color: var(--bad);
    background: var(--bad-bg);
  }
  .spin {
    flex: none;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 2px solid color-mix(in srgb, currentColor 25%, transparent);
    border-top-color: currentColor;
    animation: spin 800ms linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation: none;
    }
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
