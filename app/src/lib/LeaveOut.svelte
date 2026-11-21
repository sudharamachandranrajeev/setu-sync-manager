<script lang="ts">
  let {
    list,
    onchange,
  }: {
    /** The "always left out" entries, as the rules hold them. */
    list: string[]
    onchange: (list: string[]) => Promise<boolean>
  } = $props()

  let text = $state('')
  let open = $state(false)
  const shown = (l: string) => l.replace(/^(\(\?[di]\))+/, '')

  async function add() {
    const t = text.trim()
    if (!t) return
    if (await onchange([...list, t])) text = ''
  }
</script>

<div class="box">
  <h3>Always left out</h3>
  <p class="muted small">
    Never copied, in any folder, on either computer. Secrets are fixed: <span class="mono">API keys · .env · keystores</span>.
  </p>
  <button class="link-btn small" onclick={() => (open = !open)} aria-expanded={open}>
    {open ? 'Hide the list' : `Show and change the list (${list.length})`}
  </button>
  {#if open}
    <ul class="chips">
      {#each list as l (l)}
        <li>
          <span class="mono">{shown(l)}</span>
          <button aria-label="Stop leaving out {shown(l)}" title="Stop leaving this out" onclick={() => onchange(list.filter((x) => x !== l))}>✕</button>
        </li>
      {/each}
    </ul>
    <form
      class="add"
      onsubmit={(e) => {
        e.preventDefault()
        add()
      }}
    >
      <input type="text" placeholder="e.g. *.iso or videos" bind:value={text} aria-label="Name to leave out" />
      <button class="btn" disabled={!text.trim()}>Add</button>
    </form>
    <p class="muted small">
      Type a file or folder name, or a pattern: <span class="mono">*</span> stands for any text, so
      <span class="mono">*.mp4</span> leaves out every MP4 video. It applies at any depth. Changes go through Review like any other.
    </p>
  {/if}
</div>

<style>
  .box {
    margin: 18px 0 12px;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--surface-2);
  }
  h3 {
    margin: 0 0 4px;
    font-size: 13px;
  }
  p {
    margin: 4px 0;
    overflow-wrap: anywhere;
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
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    list-style: none;
    margin: 10px 0 8px;
    padding: 0;
  }
  .chips li {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px 2px 9px;
    border-radius: 99px;
    border: 1px solid var(--line);
    background: var(--surface);
    font-size: 12px;
  }
  .chips button {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    font-size: 10px;
    color: var(--muted);
  }
  .chips button:hover {
    background: var(--bad-bg);
    color: var(--bad);
  }
  .add {
    display: flex;
    gap: 6px;
  }
  .add input {
    flex: 1;
    min-width: 0;
  }
</style>
