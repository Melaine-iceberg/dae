import { atom } from "jotai";

import { getAppWindow } from "@/lib/app-window";

import { ExplorerNavigator } from "./navigation";

export interface ExplorerTab {
  id: string;
}

export type FileClipboard = {
  operation: "copy" | "cut";
  sourcePaths: string[];
};

const navigators = new Map<string, ExplorerNavigator>();
let nextTabId = 0;

function createTabEntry(): ExplorerTab {
  const id = `tab-${nextTabId++}`;
  navigators.set(id, new ExplorerNavigator());
  return { id };
}

export function getTabNavigator(tabId: string): ExplorerNavigator {
  const navigator = navigators.get(tabId);
  if (!navigator) {
    throw new Error(`No navigator registered for tab "${tabId}"`);
  }
  return navigator;
}

const initialTab = createTabEntry();

export const tabsAtom = atom<ExplorerTab[]>([initialTab]);
export const activeTabIdAtom = atom<string>(initialTab.id);
export const fileClipboardAtom = atom<FileClipboard | null>(null);

export const createTabAtom = atom(null, (_get, set) => {
  const tab = createTabEntry();
  set(tabsAtom, (tabs) => [...tabs, tab]);
  set(activeTabIdAtom, tab.id);
});

export const activateTabAtom = atom(null, (_get, set, tabId: string) => {
  set(activeTabIdAtom, tabId);
});

export const closeTabAtom = atom(null, (get, set, tabId: string) => {
  const tabs = get(tabsAtom);
  const index = tabs.findIndex((tab) => tab.id === tabId);
  if (index === -1) return;

  const remaining = tabs.filter((tab) => tab.id !== tabId);
  navigators.delete(tabId);

  if (remaining.length === 0) {
    getAppWindow()?.close();
    return;
  }

  set(tabsAtom, remaining);

  if (get(activeTabIdAtom) === tabId) {
    set(activeTabIdAtom, remaining[Math.min(index, remaining.length - 1)].id);
  }
});
