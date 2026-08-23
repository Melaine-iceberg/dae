import { atom } from "jotai";

/** Whether the command bar (Ctrl/Cmd+K) is open. */
export const commandBarOpenAtom = atom(false);

/**
 * Command bar flavor: `commands` (Ctrl/Cmd+K) searches commands, surfaces and
 * locations; `path` (Ctrl/Cmd+P) is the developer quick-jump — direct path
 * input plus fuzzy matching over favorites and recent directories.
 */
export type CommandBarMode = "commands" | "path";

export const commandBarModeAtom = atom<CommandBarMode>("commands");
