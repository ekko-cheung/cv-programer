<script lang="ts">
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";

  type Item = { id: number; kind: string; category: string; preview: string; blob: string | null; created: number; pinned: boolean; sync: boolean };
  type Count = { category: string; count: number };

  const TABS = [
    { id: "all", label: "All" },
    { id: "text", label: "Text" },
    { id: "code", label: "Code" },
    { id: "link", label: "Links" },
    { id: "image", label: "Images" },
    { id: "file", label: "Files" },
  ];

  let items = $state<Item[]>([]);
  let counts = $state<Record<string, number>>({});
  let tab = $state("all");
  let query = $state("");
  let sel = $state(0);
  let search: HTMLInputElement;
  let listEl: HTMLDivElement;

  async function load() {
    items = await invoke<Item[]>("list_items", { category: tab, query });
    const c = await invoke<Count[]>("category_counts");
    counts = Object.fromEntries(c.map((x) => [x.category, x.count]));
    sel = Math.min(sel, Math.max(items.length - 1, 0));
  }

  function setTab(id: string) { tab = id; sel = 0; load(); }
  function paste(i: Item) { invoke("paste_item", { id: i.id }); }

  async function toggle(i: Item, key: "pinned" | "sync") {
    await invoke(key === "pinned" ? "set_pinned" : "set_sync", { id: i.id, value: !i[key] });
    load();
  }
  async function remove(i: Item) { await invoke("delete_item", { id: i.id }); load(); }

  function ago(ms: number) {
    const s = (Date.now() - ms) / 1000;
    if (s < 60) return "now";
    if (s < 3600) return `${Math.floor(s / 60)}m`;
    if (s < 86400) return `${Math.floor(s / 3600)}h`;
    return `${Math.floor(s / 86400)}d`;
  }

  async function move(d: number) {
    sel = Math.max(0, Math.min(items.length - 1, sel + d));
    await tick();
    listEl.querySelector(".row.sel")?.scrollIntoView({ block: "nearest" });
  }

  function onkey(e: KeyboardEvent) {
    const cur = items[sel];
    if (e.key === "ArrowDown") { e.preventDefault(); move(1); }
    else if (e.key === "ArrowUp") { e.preventDefault(); move(-1); }
    else if (e.key === "Enter" && cur) { e.preventDefault(); paste(cur); }
    else if (e.key === "Escape") invoke("hide_panel");
    else if (e.key === "Tab") { e.preventDefault(); setTab(TABS[(TABS.findIndex((t) => t.id === tab) + (e.shiftKey ? TABS.length - 1 : 1)) % TABS.length].id); }
    else if ((e.metaKey || e.ctrlKey) && e.key === "p" && cur) { e.preventDefault(); toggle(cur, "pinned"); }
    else if ((e.metaKey || e.ctrlKey) && e.key === "Backspace" && cur) { e.preventDefault(); remove(cur); }
    else if ((e.metaKey || e.ctrlKey) && /^[1-9]$/.test(e.key) && items[+e.key - 1]) { e.preventDefault(); paste(items[+e.key - 1]); }
  }

  onMount(() => {
    load();
    const focus = () => { search.focus(); search.select(); };
    focus();
    // Native focus is granted asynchronously after the window is shown.
    const timers = [50, 150, 400].map((ms) => setTimeout(() => document.activeElement !== search && focus(), ms));
    // The webview may only receive focus after mount, so refocus when the window gains it.
    window.addEventListener("focus", focus);
    const un = listen("items-changed", load);
    return () => { timers.forEach(clearTimeout); window.removeEventListener("focus", focus); un.then((f) => f()); };
  });
</script>

<svelte:window onkeydown={onkey} />

<main>
  <header>
    <input bind:this={search} bind:value={query} oninput={() => { sel = 0; load(); }} placeholder="Search clipboard history…" spellcheck="false" autocomplete="off" />
  </header>
  <nav>
    {#each TABS as t}
      <button class:on={tab === t.id} onclick={() => setTab(t.id)}>
        {t.label}<span>{t.id === "all" ? Object.values(counts).reduce((a, b) => a + b, 0) : (counts[t.id] ?? 0)}</span>
      </button>
    {/each}
  </nav>
  <div class="list" bind:this={listEl}>
    {#each items as it, idx (it.id)}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="row" class:sel={idx === sel} onmousemove={() => (sel = idx)} ondblclick={() => paste(it)} onclick={() => paste(it)}>
        <span class="tag {it.category}">{it.category}</span>
        <div class="body">
          {#if it.kind === "image" && it.blob}
            <img src={convertFileSrc(it.blob)} alt={it.preview} loading="lazy" />
          {:else}
            <div class="text" class:mono={it.category === "code"}>{it.preview}</div>
          {/if}
        </div>
        <div class="meta">
          <span class="time">{ago(it.created)}</span>
          <div class="acts">
            <button title="Sync" class:active={it.sync} onclick={(e) => { e.stopPropagation(); toggle(it, "sync"); }}>☁</button>
            <button title="Pin (⌘P)" class:active={it.pinned} onclick={(e) => { e.stopPropagation(); toggle(it, "pinned"); }}>📌</button>
            <button title="Delete" onclick={(e) => { e.stopPropagation(); remove(it); }}>✕</button>
          </div>
        </div>
        {#if idx < 9}<kbd>⌘{idx + 1}</kbd>{/if}
      </div>
    {:else}
      <div class="empty">Nothing here yet — copy something.</div>
    {/each}
  </div>
  <footer><span>↑↓ select</span><span>↵ paste</span><span>⇥ category</span><span>⌘P pin</span><span>esc close</span></footer>
</main>

<style>
  main { display: flex; flex-direction: column; height: 100vh; }
  header { padding: 14px 14px 8px; }
  input { width: 100%; padding: 10px 14px; border-radius: 10px; border: 1px solid var(--line); background: var(--chip); outline: none; user-select: text; }
  input:focus { border-color: var(--accent); }
  nav { display: flex; gap: 4px; padding: 0 14px 8px; border-bottom: 1px solid var(--line); }
  nav button { padding: 5px 10px; border-radius: 8px; color: var(--muted); display: flex; gap: 6px; align-items: center; }
  nav button span { font-size: 11px; opacity: 0.7; }
  nav button:hover { background: var(--hover); }
  nav button.on { background: var(--accent); color: var(--accent-fg); }
  .list { flex: 1; overflow-y: auto; padding: 6px; }
  .row { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: 10px; cursor: default; min-height: 44px; }
  .row.sel { background: var(--hover); }
  .tag { font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em; padding: 2px 7px; border-radius: 6px; background: var(--chip); color: var(--muted); width: 48px; text-align: center; flex: none; }
  .tag.code { color: #2f9e6e; } .tag.link { color: #2f7be0; } .tag.image { color: #d9822b; } .tag.file { color: #b04fd0; }
  .body { flex: 1; min-width: 0; }
  .text { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; word-break: break-all; white-space: pre-wrap; max-height: 2.8em; }
  .mono { font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 12px; }
  img { max-height: 72px; max-width: 100%; border-radius: 6px; display: block; }
  .meta { display: flex; flex-direction: column; align-items: flex-end; gap: 2px; flex: none; }
  .time { font-size: 11px; color: var(--muted); }
  .acts { display: none; gap: 2px; }
  .row.sel .acts { display: flex; } .row.sel .time { display: none; }
  .acts button { padding: 2px 5px; border-radius: 6px; opacity: 0.5; font-size: 12px; }
  .acts button:hover, .acts button.active { opacity: 1; background: var(--chip); }
  kbd { font: 11px ui-monospace, monospace; color: var(--muted); flex: none; width: 28px; text-align: right; }
  .empty { text-align: center; color: var(--muted); padding: 60px 0; }
  footer { display: flex; gap: 14px; justify-content: center; padding: 8px; border-top: 1px solid var(--line); color: var(--muted); font-size: 11px; }
</style>
