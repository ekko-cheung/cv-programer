export type Appearance = { glass: boolean; glass_opacity: number; font_ui: string; font_mono: string };

/** Applies the user's font choices on top of the defaults declared in app.css. */
export function applyFonts(a: Pick<Appearance, "font_ui" | "font_mono">) {
  const root = document.documentElement.style;
  const set = (name: string, user: string) =>
    user.trim() ? root.setProperty(name, `${user.trim()}, var(${name}-default)`) : root.removeProperty(name);
  set("--font-ui", a.font_ui);
  set("--font-mono", a.font_mono);
}
