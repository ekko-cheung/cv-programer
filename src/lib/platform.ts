export const isMac = navigator.userAgent.includes("Mac");

/** Primary shortcut modifier shown in the UI: ⌘ on macOS, Ctrl on Windows. */
export const mod = isMac ? "⌘" : "Ctrl";

/** Joins modifier + key the way each platform writes it: "⌘P" vs "Ctrl+P". */
export const combo = (key: string) => (isMac ? `${mod}${key}` : `${mod}+${key}`);

const names: Record<string, string> = isMac
  ? { Super: "⌘", Control: "⌃", Alt: "⌥", Shift: "⇧" }
  : { Super: "Win", Control: "Ctrl", Alt: "Alt", Shift: "Shift" };

/** Renders an accelerator like "Super+Shift+V" with platform-specific glyphs. */
export const prettyAccelerator = (acc: string) =>
  acc.split("+").map((p) => names[p] ?? p).join(isMac ? " " : " + ");
