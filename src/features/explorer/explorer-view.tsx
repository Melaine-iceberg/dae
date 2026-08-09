import { useEffect, useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  ArrowUpIcon,
  RefreshCwIcon,
  TriangleAlertIcon,
} from "lucide-react";

import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

import { explorerApi } from "./api";
import { ExplorerBreadcrumbs } from "./explorer-breadcrumbs";
import { FileList, FileListSkeleton } from "./file-list";
import type { ExplorerNavigator } from "./navigation";

const DIRECTORY_CHANGED_EVENT = "explorer-directory-changed";
const DIRECTORY_REFRESH_DELAY_MS = 150;
const appWindow = getCurrentWindow();

interface ExplorerViewProps {
  navigator: ExplorerNavigator;
}

export function ExplorerView({ navigator }: ExplorerViewProps) {
  const state = useSyncExternalStore(navigator.subscribe, navigator.getSnapshot);
  const directory = state.directory;
  const directoryPath = directory?.path;
  const isLoading = state.status === "loading";
  const canGoBack = !isLoading && state.historyIndex > 0;
  const canGoForward = !isLoading && state.historyIndex < state.history.length - 1;
  const canGoUp = !isLoading && (directory?.breadcrumbs.length ?? 0) > 1;

  useEffect(() => {
    if (navigator.getSnapshot().status === "idle") {
      void navigator.initialize();
    }
  }, [navigator]);

  useEffect(() => {
    if (!directoryPath) return;

    void explorerApi
      .watchDirectory(directoryPath)
      .then(() => void navigator.refresh(directoryPath))
      .catch((error: unknown) => {
        console.warn("Unable to watch directory for changes", error);
      });
  }, [navigator, directoryPath]);

  useEffect(() => {
    let disposed = false;
    let refreshTimeout: number | undefined;

    const scheduleRefresh = (path: string) => {
      if (disposed || navigator.getSnapshot().directory?.path !== path) return;

      window.clearTimeout(refreshTimeout);
      refreshTimeout = window.setTimeout(() => {
        refreshTimeout = undefined;
        void navigator.refresh(path);
      }, DIRECTORY_REFRESH_DELAY_MS);
    };

    const unlistenChangesPromise = listen<string>(DIRECTORY_CHANGED_EVENT, ({ payload }) => {
      scheduleRefresh(payload);
    });
    const unlistenFocusPromise = appWindow.onFocusChanged(({ payload: focused }) => {
      const currentPath = navigator.getSnapshot().directory?.path;
      if (focused && currentPath) scheduleRefresh(currentPath);
    });

    return () => {
      disposed = true;
      window.clearTimeout(refreshTimeout);
      void Promise.all([unlistenChangesPromise, unlistenFocusPromise]).then((unlisten) => {
        unlisten.forEach((stopListening) => stopListening());
      });
    };
  }, [navigator]);

  const retry = () => {
    if (directory) {
      void navigator.navigate(directory.path);
      return;
    }

    void navigator.initialize();
  };

  return (
    <main className="h-full bg-background">
      <section className="flex h-full w-full flex-col overflow-hidden">
        <header
          className="flex h-14 shrink-0 items-center gap-1 border-b px-2 sm:px-3"
          data-tauri-drag-region
        >
          <div className="flex shrink-0 items-center gap-0.5">
            <Button
              aria-label="后退"
              disabled={!canGoBack}
              onClick={() => void navigator.goBack()}
              size="icon-sm"
              title="后退"
              type="button"
              variant="ghost"
            >
              <ArrowLeftIcon />
            </Button>
            <Button
              aria-label="前进"
              disabled={!canGoForward}
              onClick={() => void navigator.goForward()}
              size="icon-sm"
              title="前进"
              type="button"
              variant="ghost"
            >
              <ArrowRightIcon />
            </Button>
            <Button
              aria-label="上一级"
              disabled={!canGoUp}
              onClick={() => void navigator.goUp()}
              size="icon-sm"
              title="上一级"
              type="button"
              variant="ghost"
            >
              <ArrowUpIcon />
            </Button>
            <Button
              aria-label="刷新"
              disabled={isLoading || !directory}
              onClick={() => directory && void navigator.navigate(directory.path)}
              size="icon-sm"
              title="刷新"
              type="button"
              variant="ghost"
            >
              <RefreshCwIcon className={cn(isLoading && "animate-spin")} />
            </Button>
          </div>

          <div className="min-w-0 flex-1 px-2" data-tauri-drag-region>
            {directory ? (
              <ExplorerBreadcrumbs
                breadcrumbs={directory.breadcrumbs}
                onNavigate={(breadcrumb) => void navigator.navigateBreadcrumb(breadcrumb)}
              />
            ) : (
              <Skeleton className="h-4 w-48 max-w-full" />
            )}
          </div>
        </header>

        {state.error && directory && (
          <div className="shrink-0 p-3 pb-0">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        )}

        {directory ? (
          <FileList
            entries={directory.entries}
            initialScrollOffset={navigator.getScrollOffset(directory.path)}
            isLoading={isLoading}
            onOpenDirectory={(path) => void navigator.navigate(path)}
            onScrollOffsetChange={(offset) => navigator.setScrollOffset(directory.path, offset)}
          />
        ) : state.error ? (
          <div className="p-4">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        ) : (
          <FileListSkeleton />
        )}
      </section>
    </main>
  );
}

function ExplorerErrorAlert({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <Alert variant="destructive">
      <TriangleAlertIcon />
      <AlertTitle>无法读取此位置</AlertTitle>
      <AlertDescription>{message}</AlertDescription>
      <AlertAction>
        <Button onClick={onRetry} size="xs" type="button" variant="outline">
          重试
        </Button>
      </AlertAction>
    </Alert>
  );
}
