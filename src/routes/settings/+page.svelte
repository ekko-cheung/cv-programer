<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { isMac, prettyAccelerator as pretty } from "$lib/platform";

  type S3 = { endpoint: string; region: string; bucket: string; prefix: string; access_key: string; path_style: boolean };
  type Settings = { hotkey: string; max_items: number; s3: S3 };

  let saved = $state<Settings | null>(null);
  let form = $state<Settings>({ hotkey: "", max_items: 2000, s3: { endpoint: "", region: "", bucket: "", prefix: "", access_key: "", path_style: false } });
  let secret = $state("");
  let hasSecret = $state(false);
  let testing = $state(false);
  let recording = $state(false);
  let status = $state<{ ok: boolean; msg: string } | null>(null);
  let confirmClear = $state(false);

  const dirty = $derived(saved !== null && (JSON.stringify(saved) !== JSON.stringify(form) || secret !== ""));

  onMount(async () => {
    saved = await invoke<Settings>("get_settings");
    form = structuredClone($state.snapshot(saved));
    hasSecret = await invoke<boolean>("has_s3_secret");
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
      if (secret !== "") {
        await invoke("set_s3_secret", { secret });
        hasSecret = true;
        secret = "";
      }
      saved = await invoke<Settings>("save_settings", { settings: form });
      form = structuredClone($state.snapshot(saved));
      status = { ok: true, msg: "Saved." };
    } catch (e) {
      status = { ok: false, msg: String(e) };
    }
  }

  async function testS3() {
    testing = true;
    status = { ok: true, msg: "Testing connection…" };
    try {
      await invoke("test_s3", { config: form.s3, secret: secret || null });
      status = { ok: true, msg: "Connection OK — bucket is readable and writable." };
    } catch (e) {
      status = { ok: false, msg: String(e) };
    } finally {
      testing = false;
    }
  }

  async function removeSecret() {
    await invoke("set_s3_secret", { secret: "" });
    hasSecret = false;
    secret = "";
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
    <h2>S3 storage</h2>
    <p class="hint block">Items marked with ☁ are synced to this bucket. Works with AWS S3, Cloudflare R2, MinIO, Aliyun OSS and other S3-compatible services.</p>

    <label class="f"><span>Endpoint</span><input class="wide" bind:value={form.s3.endpoint} placeholder="https://… (empty for AWS S3)" spellcheck="false" /></label>
    <label class="f"><span>Region</span><input class="wide" bind:value={form.s3.region} placeholder="us-east-1 (R2: auto)" spellcheck="false" /></label>
    <label class="f"><span>Bucket</span><input class="wide" bind:value={form.s3.bucket} spellcheck="false" /></label>
    <label class="f"><span>Path prefix</span><input class="wide" bind:value={form.s3.prefix} placeholder="cv-programer/" spellcheck="false" /></label>
    <label class="f"><span>Access key ID</span><input class="wide" bind:value={form.s3.access_key} spellcheck="false" autocomplete="off" /></label>
    <label class="f">
      <span>Secret access key</span>
      <input class="wide" type="password" bind:value={secret} autocomplete="off" placeholder={hasSecret ? "•••••••• (saved in system keychain)" : ""} />
    </label>
    <label class="f check"><span>Path-style URLs</span><input type="checkbox" bind:checked={form.s3.path_style} /></label>

    <div class="row end">
      {#if hasSecret}<button onclick={removeSecret}>Remove saved secret</button>{/if}
      <button disabled={testing} onclick={testS3}>Test connection</button>
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
  .row.end { justify-content: flex-end; padding: 8px 0 4px; }
  .hint.block { max-width: none; margin: 0 0 8px; }
  .f { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 5px 0; }
  .f > span { color: var(--fg); }
  .f input.wide { width: 300px; text-align: left; }
  .f.check input { width: auto; }
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
