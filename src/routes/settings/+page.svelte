<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { isMac, prettyAccelerator as pretty } from "$lib/platform";

  type Settings = { hotkey: string; max_items: number };

  let saved = $state<Settings | null>(null);
  let form = $state<Settings>({ hotkey: "", max_items: 2000 });
  let recording = $state(false);
  let status = $state<{ ok: boolean; msg: string } | null>(null);
  let confirmClear = $state(false);

  const dirty = $derived(saved !== null && (saved.hotkey !== form.hotkey || saved.max_items !== form.max_items));

  onMount(async () => {
    saved = await invoke<Settings>("get_settings");
    form = { ...saved };
  });

  function keyName(e: KeyboardEvent): string | null {
    if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
    if (/^Digit\d$/.test(e.code)) return e.code.slice(5);
    if (/^F\d{1,2}$/.test(e.code)) return e.code;
    const map: Record<string, string> = { Space: "Space", Enter: "Enter", Tab: "Tab", Backquote: "`", Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]", Semicolon: ";", Quote: "'", Comma: ",", Period: ".", Slash: "/", Backslash: "\\" };
    return map[e.code] ?? null;
  }

  function onRecord(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") { recording = false; return; }
    const key = keyName(e);
    if (!key) return; // modifier-only press so far
    const mods = [e.metaKey && "Super", e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift"].filter(Boolean) as string[];
    if (!mods.length) { status = { ok: false, msg: "Include at least one modifier key (" + (isMac ? "⌘ / ⌃ / ⌥ / ⇧" : "Ctrl / Alt / Shift / Win") + ")." }; return; }
    form.hotkey = [...mods, key].join("+");
    recording = false;
    status = null;
  }

  async function save() {
    try {
      saved = await invoke<Settings>("save_settings", { settings: form });
      form = { ...saved };
      status = { ok: true, msg: "Saved." };
    } catch (e) {
      status = { ok: false, msg: String(e) };
    }
  }

  async function clear(keepPinned: boolean) {
    await invoke("clear_history", { keepPinned });
    confirmClear = false;
    status = { ok: true, msg: keepPinned ? "History cleared (pinned items kept)." : "All history cleared." };
  }
</script>

<svelte:window onkeydown={onRecord} />

<main>
  <h1>Settings</h1>

  <section>
    <h2>General</h2>

    <div class="field">
      <div>
        <div class="label">Open clipboard panel</div>
        <div class="hint">Global shortcut to show or hide the panel.</div>
      </div>
      <button class="keys" class:rec={recording} onclick={() => { recording = true; status = null; }} onblur={() => (recording = false)}>
        {recording ? "Press shortcut…" : pretty(form.hotkey)}
      </button>
    </div>

    <div class="field">
      <div>
        <div class="label">History size</div>
        <div class="hint">Unpinned items to keep (50 – 20,000). Older ones are removed.</div>
      </div>
      <input type="number" min="50" max="20000" step="50" bind:value={form.max_items} />
    </div>
  </section>

  <section>
    <h2>Data</h2>
    <div class="field">
      <div>
        <div class="label">Clear history</div>
        <div class="hint">Removes saved items and cached images from this device.</div>
      </div>
      {#if confirmClear}
        <div class="row">
          <button onclick={() => clear(true)}>Keep pinned</button>
          <button class="danger" onclick={() => clear(false)}>Delete all</button>
          <button onclick={() => (confirmClear = false)}>Cancel</button>
        </div>
      {:else}
        <button class="danger-o" onclick={() => (confirmClear = true)}>Clear…</button>
      {/if}
    </div>
  </section>

  <footer>
    <span class:err={status && !status.ok} class="status">{status?.msg ?? ""}</span>
    <button class="primary" disabled={!dirty} onclick={save}>Save</button>
  </footer>
</main>

<style>
  main { padding: 24px 28px; height: 100vh; display: flex; flex-direction: column; overflow-y: auto; }
  h1 { font-size: 20px; margin: 0 0 18px; }
  h2 { font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); margin: 0 0 8px; }
  section { margin-bottom: 22px; border-bottom: 1px solid var(--line); padding-bottom: 6px; }
  section:last-of-type { border-bottom: 0; }
  .field { display: flex; justify-content: space-between; align-items: center; gap: 20px; padding: 10px 0; }
  .label { font-weight: 500; }
  .hint { color: var(--muted); font-size: 12px; margin-top: 2px; max-width: 300px; }
  .row { display: flex; gap: 6px; }
  button, input { padding: 6px 12px; border-radius: 8px; border: 1px solid var(--line); background: var(--chip); }
  input { width: 100px; text-align: right; outline: none; user-select: text; }
  input:focus, .keys.rec { border-color: var(--accent); }
  .keys { min-width: 130px; font-weight: 500; letter-spacing: 0.04em; }
  .keys.rec { color: var(--accent); }
  button:hover:not(:disabled) { background: var(--hover); }
  .danger-o { color: #e5484d; }
  .danger { background: #e5484d; color: #fff; border-color: #e5484d; }
  .danger:hover:not(:disabled) { background: #d23a3f; }
  footer { margin-top: auto; display: flex; justify-content: space-between; align-items: center; padding-top: 12px; }
  .status { font-size: 12px; color: var(--muted); }
  .status.err { color: #e5484d; }
  .primary { background: var(--accent); color: var(--accent-fg); border-color: var(--accent); }
  .primary:hover:not(:disabled) { background: var(--accent); filter: brightness(1.1); }
  .primary:disabled { opacity: 0.4; cursor: default; }
</style>
