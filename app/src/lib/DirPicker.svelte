<script lang="ts">
  import { glyph, shortLabel, type Dir } from './format'

  let {
    value,
    allowed = ['off', 'both', 'out', 'in'],
    meName,
    otherName,
    onpick,
  }: {
    value: Dir
    allowed?: Dir[]
    meName: string
    otherName: string
    onpick: (d: Dir) => void
  } = $props()

  const order: Dir[] = ['off', 'out', 'both', 'in']
  const title = (d: Dir) =>
    ({ off: 'Off', out: `${meName} → ${otherName}`, both: 'Both ways', in: `${otherName} → ${meName}` })[d]
</script>

<div class="seg" role="radiogroup" aria-label="Direction">
  {#each order as d (d)}
    <button
      role="radio"
      aria-checked={value === d}
      class="opt {d}"
      class:on={value === d}
      disabled={!allowed.includes(d)}
      title={title(d)}
      onclick={() => onpick(d)}
    >
      <span class="g">{glyph[d]}</span>
      <span class="t">{shortLabel(d)}</span>
    </button>
  {/each}
</div>

<style>
  .seg {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 4px;
    padding: 4px;
    border-radius: 12px;
    background: var(--surface-2);
    border: 1px solid var(--line);
  }
  .opt {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 4px;
    border-radius: 9px;
    color: var(--muted);
    transition:
      background 140ms ease,
      color 140ms ease,
      transform 100ms ease;
  }
  .opt:active:not(:disabled) {
    transform: scale(0.97);
  }
  .opt:hover:not(:disabled):not(.on) {
    background: var(--surface);
    color: var(--text);
  }
  .g {
    font-size: 18px;
    line-height: 1;
  }
  .t {
    font-size: 12px;
    font-weight: 600;
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
