<script lang="ts">
  import { choiceLabel, glyph, type Dir } from './format'

  let {
    value,
    allowed = ['off', 'both', 'out', 'in'],
    there,
    onpick,
  }: {
    value: Dir
    allowed?: Dir[]
    /** Everyday name of the other computer. */
    there: string
    onpick: (d: Dir) => void
  } = $props()

  const order: Dir[] = ['in', 'out', 'both', 'off']
  const hint = (d: Dir) =>
    ({
      in: `${there} → this computer`,
      out: `This computer → ${there}`,
      both: 'Same files on both',
      off: 'Leave it out',
    })[d]
</script>

<div class="list" role="radiogroup" aria-label="How to sync">
  {#each order as d (d)}
    <button
      role="radio"
      aria-checked={value === d}
      class="opt {d}"
      class:on={value === d}
      disabled={!allowed.includes(d)}
      onclick={() => onpick(d)}
    >
      <span class="g" aria-hidden="true">{glyph[d]}</span>
      <span class="t">{choiceLabel(d, there)}</span>
      <span class="h">{hint(d)}</span>
    </button>
  {/each}
</div>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px;
    border-radius: 12px;
    background: var(--surface-2);
    border: 1px solid var(--line);
  }
  .opt {
    display: grid;
    grid-template-columns: 24px 1fr;
    grid-template-rows: auto auto;
    column-gap: 8px;
    align-items: center;
    padding: 7px 10px;
    border-radius: 9px;
    text-align: left;
    color: var(--muted);
    transition:
      background 140ms ease,
      color 140ms ease;
  }
  .opt:disabled {
    opacity: 0.45;
  }
  .opt:hover:not(:disabled):not(.on) {
    background: var(--surface);
    color: var(--text);
  }
  .g {
    grid-row: span 2;
    font-size: 18px;
    line-height: 1;
    text-align: center;
  }
  .t {
    font-size: 13px;
    font-weight: 650;
  }
  .h {
    font-size: 11.5px;
    opacity: 0.85;
  }
  .on {
    background: var(--surface);
    box-shadow: var(--shadow);
  }
  .on.off {
    color: var(--off);
  }
  .on.both {
    color: var(--both);
  }
  .on.out {
    color: var(--out);
  }
  .on.in {
    color: var(--in);
  }
</style>
