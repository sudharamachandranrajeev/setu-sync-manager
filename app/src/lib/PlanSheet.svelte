<script lang="ts">
  import type { DeviceView, PlanView } from './api'
  import { dirOf, glyph, shortLabel } from './format'

  let {
    view,
    me,
    other,
    busy,
    onclose,
    onapply,
  }: {
    view: PlanView
    me: DeviceView
    other: DeviceView
    busy: boolean
    onclose: () => void
    onapply: () => void
  } = $props()

  const plan = $derived(view.plan)
  const blocked = $derived(plan.problems.length > 0)
  const nothing = $derived(plan.actions.length === 0 && view.edits.length === 0)
  const typeLabel = (t: string) => ({ sendonly: 'sends only', receiveonly: 'receives only', sendreceive: 'both ways' })[t] ?? t
  const where = (p: string) => p || '(whole folder)'
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-labelledby="plan-title">
  <header>
    <h2 id="plan-title">Review changes</h2>
    <p class="muted">Nothing changes until you press Apply. Files are never deleted by applying.</p>
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
        <h3>Your choices</h3>
        <ul class="edits">
          {#each view.edits as e (e.path)}
            {@const b = dirOf(e.before, me.id)}
            {@const a = dirOf(e.after, me.id)}
            <li>
              <span class="mono path">{where(e.path)}</span>
              <span class="chip {b}" class:inherit={e.before === null}>{glyph[b]} {e.before === null ? 'inherited' : shortLabel(b)}</span>
              <span class="arrow">→</span>
              <span class="chip {a}" class:inherit={e.after === null}>{glyph[a]} {e.after === null ? 'inherited' : shortLabel(a)}</span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section>
      <h3>Syncthing on {me.name}</h3>
      {#if plan.actions.length === 0}
        <p class="muted">No changes here. {plan.unchanged} Setu folder(s) already match.</p>
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
    </section>

    {#if plan.warnings.length}
      <section class="box warn">
        {#each plan.warnings as w, i (i)}<p>{w}</p>{/each}
      </section>
    {/if}

    <p class="muted small">
      After you apply, {other.name} applies revision {plan.revision} on its side. A folder starts
      syncing once both computers have it.
    </p>
  </div>

  <footer>
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={blocked || busy || nothing} onclick={onapply}>
      {busy ? 'Applying…' : `Apply revision ${plan.revision}`}
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
