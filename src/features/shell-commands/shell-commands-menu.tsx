/**
 * The "应用扩展" submenu: the third-party right-click commands the installed
 * apps declare, hosted through whichever mechanism the platform provides
 * (Windows shell verbs, macOS Services, Linux KDE service menus).
 *
 * Commands arrive the way Explorer arranges them — one row each, except that an
 * app contributing more than one command gets its own nested submenu named after
 * it, so two rows from the same app never read as two unrelated tools. The other
 * platforms have no grouping convention of their own, so this is the shape they
 * get too, which is what keeps the section recognisable across all three.
 *
 * Everything inside comes from the platform through `shell-commands-atoms`,
 * including the wording: a command's label is whatever its app decided to call
 * it in the user's language, which is why none of these rows are translated.
 */

import { Fragment } from "react";
import { useTranslation } from "react-i18next";
import { Boxes } from "lucide-react";

import { invokeShellCommand, useShellCommands } from "./shell-commands-atoms";

import type { ShellCommand } from "@/bindings";
import {
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu";

interface ShellCommandsMenuProps {
  /** Paths the command runs on — the selection, or just the clicked entry. */
  paths: readonly string[];
  /** The right-clicked entry, which decides the commands that apply. */
  primary: string;
  /** Receives a failure message, or `null` when the command ran. */
  onError: (message: string | null) => void;
}

/**
 * Renders the submenu, separator included. A selection with no applicable
 * commands — none installed, a remote one, or a first reply still in flight —
 * renders nothing at all: no empty submenu to open, and no stray separator
 * either, so the rest of the menu keeps its layout.
 */
export function ShellCommandsMenu({ paths, primary, onError }: ShellCommandsMenuProps) {
  const { t } = useTranslation("explorer");
  const items = useShellCommands(paths, primary);
  if (items.length === 0) return null;

  const inline = items.filter((item) => item.group === null);
  const grouped = new Map<string, ShellCommand[]>();
  for (const item of items) {
    if (item.group === null) continue;
    grouped.set(item.group, [...(grouped.get(item.group) ?? []), item]);
  }

  return (
    <>
      <ContextMenuSeparator />
      <ContextMenuSub>
        <ContextMenuSubTrigger>
          <Boxes />
          {t("explorer:shellCommands.menu")}
        </ContextMenuSubTrigger>
        <ContextMenuSubContent>
          {inline.map((item, index) => (
            <Fragment key={item.id}>
              {item.separatorBefore && index > 0 && <ContextMenuSeparator />}
              <ShellCommandItem item={item} paths={paths} onError={onError} />
            </Fragment>
          ))}
          {[...grouped].map(([app, appItems]) => (
            <ContextMenuSub key={app}>
              <ContextMenuSubTrigger>
                <ShellCommandIcon item={appItems[0]} />
                {app}
              </ContextMenuSubTrigger>
              <ContextMenuSubContent>
                {appItems.map((item) => (
                  <ShellCommandItem key={item.id} item={item} paths={paths} onError={onError} />
                ))}
              </ContextMenuSubContent>
            </ContextMenuSub>
          ))}
        </ContextMenuSubContent>
      </ContextMenuSub>
    </>
  );
}

function ShellCommandItem({
  item,
  paths,
  onError,
}: {
  item: ShellCommand;
  paths: readonly string[];
  onError: (message: string | null) => void;
}) {
  return (
    <ContextMenuItem
      disabled={item.disabled}
      onClick={() => {
        onError(null);
        void invokeShellCommand(item.id, paths).then(onError);
      }}
    >
      <ShellCommandIcon item={item} />
      {/* One child, not several: the item is a flex row with a gap, so a label
          split across nodes would be laid out with a gap between its pieces. */}
      <span>{item.label}</span>
    </ContextMenuItem>
  );
}

function ShellCommandIcon({ item }: { item: ShellCommand }) {
  if (!item.iconDataUrl) return <Boxes />;
  return <img alt="" className="size-4 shrink-0" src={item.iconDataUrl} />;
}
