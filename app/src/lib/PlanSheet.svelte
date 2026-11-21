<script lang="ts">
  import type { DeviceView, PlanView } from './api'
  import { bytes, dirOf, glyph, shortLabel } from './format'

  let {
    view,
    me,
    there,
    busy,
    onclose,
    onapply,
  }: {
    view: PlanView
    me: DeviceView
    /** Everyday name of the other computer. */
    there: string
    busy: boolean
    onclose: () => void
    onapply: (guardConfirmed: boolean) => void
  } = $props()

  const plan = $derived(view.plan)
  const pv = $derived(view.preview)
  let understood = $state(false)
  const guarded = $derived(pv.guard.length > 0)
  const ruleName = (g: { rel: string }) => g.rel || 'Everything else'
  const modeText = (m: string) => {
    const d = dirOf(m, me.id)
    return d === 'out' ? `this computer → ${there}` : d === 'in' ? `${there} → this computer` : d === 'both' ? 'both ways' : shortLabel(d, there)
  }
  const blocked = $derived(plan.problems.length > 0)
  const nothing = $derived(plan.actions.length === 0 && view.edits.length === 0)
  const typeLabel = (t: string) => ({ sendonly: 'sends only', receiveonly: 'receives only', sendreceive: 'both ways' })[t] ?? t
  const where = (p: string) => p || 'Everything (top level)'
  const files = (n: number) => `${n.toLocaleString()} file${n === 1 ? '' : 's'}`
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-labelledby="plan-title">
  <header>
    <h2 id="plan-title">Check before syncing</h2>
    <p class="muted">Nothing changes until you press Start syncing. Pressing it does not delete any file.</p>
  </header>

  <div class="body">
    {#if blocked}
      <section class="box bad">
        <h3>Fix these first</h3>
        {#each plan.problems as p, i (i)}
          <p><b>{where(p.path)}</b>: {p.message}</p>
        {/each}
      </section>
    {/if}

    {#if view.edits.length}
      <section>
        <h3>What you changed</h3>
        <ul class="edits">
          {#each view.edits as e (e.path)}
            {@const b = dirOf(e.before, me.id)}
            {@const a = dirOf(e.after, me.id)}
            <li>
              <span class="mono path">{where(e.path)}</span>
              <span class="chip {b}" class:inherit={e.before === null}>{glyph[b]} {e.before === null ? 'follows folder' : shortLabel(b, there)}</span>
              <span class="arrow">→</span>
              <span class="chip {a}" class:inherit={e.after === null}>{glyph[a]} {e.after === null ? 'follows folder' : shortLabel(a, there)}</span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <details class="tech">
      <summary class="small">Technical details: Syncthing folders on this computer</summary>
      {#if plan.actions.length === 0}
        <p class="muted small">No changes. {plan.unchanged} Syncthing folder(s) already match.</p>
      {:else}
        <ul class="acts">
          {#each plan.actions as a (a.kind + a.id)}
            <li>
              <span class="tag {a.kind}">{a.kind === 'add' ? 'Add' : a.kind === 'update' ? 'Change' : 'Remove'}</span>
              <span class="mono path" title={a.path}>{a.path}</span>
              <span class="muted small">
                {#if a.kind === 'add'}{typeLabel(a.folderType)}{a.filesOnly ? ', chosen files only' : ''}
                {:else if a.kind === 'update'}{a.changes.join(', ')}
                {:else}stops syncing; files stay{/if}
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    </details>

    <section>
      <h3>What will happen</h3>
      {#if pv.otherUnknown}
        <p class="muted small">{there}'s file list has not arrived yet, so only this computer's files are counted.</p>
      {/if}
      {#if pv.groups.length === 0}
        <p class="muted">Nothing will sync. Every folder is set to Don't sync.</p>
      {/if}
      {#each pv.groups as g (g.rel)}
        <div class="group">
          <div class="ghead"><span class="mono path">{ruleName(g)}</span><span class="muted small">{modeText(g.mode)}</span></div>
          <div class="flows">
            {#if g.send.files}<span class="flow out">→ copy {files(g.send.files)} ({bytes(g.send.bytes)}) to {there}</span>{/if}
            {#if g.receive.files}<span class="flow in">← copy {files(g.receive.files)} ({bytes(g.receive.bytes)}) from {there}</span>{/if}
            {#if g.replacedThere.files}<span class="flow rep">{files(g.replacedThere.files)} on {there} get this computer's version</span>{/if}
            {#if g.replacedHere.files}<span class="flow rep">{files(g.replacedHere.files)} here get {there}'s version</span>{/if}
            {#if g.staysHere.files}<span class="flow">{files(g.staysHere.files)} only on this computer: kept, not copied</span>{/if}
            {#if g.staysThere.files}<span class="flow">{files(g.staysThere.files)} only on {there}: kept, not copied</span>{/if}
            {#if g.same.files}<span class="flow">{files(g.same.files)} already the same</span>{/if}
          </div>
          {#if g.samples.length}
            <details>
              <summary class="small">Show files that get the other version{g.samples.length >= 200 ? ' (first 200)' : ''}</summary>
              <ul class="samples mono small">
                {#each g.samples as it (it.path)}
                  <li>{it.path} <span class="muted">— {typeof it.change === 'object' && it.change.replace.winnerHere ? 'this computer\'s version is used' : `${there}'s version is used`}</span></li>
                {/each}
              </ul>
            </details>
          {/if}
        </div>
      {/each}
      {#if pv.groups.some((g) => g.replacedHere.files || g.replacedThere.files)}
        <p class="muted small">Nothing is lost: the version that loses is kept next to the file with <span class="mono">sync-conflict</span> in its name. Pick which to keep later under Conflicts.</p>
      {/if}
    </section>

    {#if pv.windowsNames.length}
      <section class="box warn">
        <h3>Windows can't use these names</h3>
        <p class="small">These won't copy to {there} until you rename them. Everything else still syncs.</p>
        <ul class="samples mono small">
          {#each pv.windowsNames.slice(0, 50) as n (n.path + n.message)}<li><b>{n.path}</b>: {n.message}</li>{/each}
        </ul>
        {#if pv.windowsNames.length > 50}<p class="small">…and {pv.windowsNames.length - 50} more.</p>{/if}
      </section>
    {/if}

    {#if guarded}
      <section class="box bad">
        <h3>Check this before applying</h3>
        {#each pv.guard as g, i (i)}<p>{g}</p>{/each}
        <label class="ack"><input type="checkbox" bind:checked={understood} /> I have read this and want to go ahead</label>
      </section>
    {/if}

    {#if plan.warnings.length}
      <section class="box warn">
        {#each plan.warnings as w, i (i)}<p>{w}</p>{/each}
      </section>
    {/if}

    <p class="next small">
      <b>Next step:</b> open Setu on {there} and press <i>Start syncing</i> there too. Copying starts once both computers have
      said yes.
    </p>
  </div>

  <footer>
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={blocked || busy || nothing || (guarded && !understood)} onclick={() => onapply(understood)}>
      {busy ? 'Saving…' : 'Start syncing'}
    </button>
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
    width: min(720px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 16px;
    box-shadow: var(--shadow);
    animation: rise 200ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  header {
    padding: 20px 24px 8px;
  }
  h2 {
    margin: 0 0 4px;
    font-size: 18px;
  }
  header p {
    margin: 0;
  }
  .body {
    padding: 8px 24px;
    overflow: auto;
  }
  h3 {
    margin: 16px 0 8px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--faint);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 0;
    border-bottom: 1px solid var(--line);
    min-width: 0;
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .small {
    font-size: 12.5px;
  }
  .chip {
    font-size: 12px;
    font-weight: 600;
    padding: 2px 9px;
    border-radius: 99px;
    background: color-mix(in srgb, currentColor 12%, transparent);
    white-space: nowrap;
  }
  .chip.inherit {
    background: none;
    border: 1px dashed currentColor;
  }
  .arrow {
    color: var(--faint);
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
  .tag {
    flex: none;
    width: 64px;
    text-align: center;
    font-size: 11.5px;
    font-weight: 700;
    padding: 2px 0;
    border-radius: 6px;
  }
  .tag.add {
    color: var(--ok);
    background: color-mix(in srgb, var(--ok) 14%, transparent);
  }
  .tag.update {
    color: var(--out);
    background: color-mix(in srgb, var(--out) 14%, transparent);
  }
  .tag.remove {
    color: var(--warn);
    background: color-mix(in srgb, var(--warn) 16%, transparent);
  }
  .box {
    margin-top: 14px;
    padding: 10px 14px;
    border-radius: 10px;
  }
  .box p {
    margin: 4px 0;
  }
  .box h3 {
    margin-top: 2px;
    color: inherit;
  }
  .bad {
    background: var(--bad-bg);
    color: var(--bad);
  }
  .warn {
    background: var(--warn-bg);
    color: var(--warn);
  }
  .group {
    padding: 10px 0;
    border-bottom: 1px solid var(--line);
  }
  .ghead {
    display: flex;
    gap: 10px;
    align-items: baseline;
  }
  .flows {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .flow {
    font-size: 12px;
    padding: 2px 9px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--muted);
  }
  .flow.out {
    color: var(--out);
  }
  .flow.in {
    color: var(--in);
  }
  .flow.rep {
    color: var(--warn);
    background: color-mix(in srgb, var(--warn) 14%, transparent);
  }
  details {
    margin-top: 6px;
  }
  summary {
    cursor: pointer;
    color: var(--accent);
  }
  .samples {
    max-height: 180px;
    overflow: auto;
    margin-top: 6px;
  }
  .samples li {
    display: block;
    padding: 2px 0;
    border: none;
    overflow-wrap: anywhere;
  }
  .ack {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 8px;
    font-weight: 600;
  }
  .tech {
    margin-top: 14px;
  }
  .next {
    margin: 16px 0 8px;
    padding: 10px 14px;
    border-radius: 10px;
    background: var(--surface-2);
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
</style>
