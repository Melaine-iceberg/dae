import { useEffect, useSyncExternalStore } from "react";
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

import { ExplorerBreadcrumbs } from "./explorer-breadcrumbs";
import { FileList, FileListSkeleton } from "./file-list";
import { explorerNavigator } from "./navigation";

export function ExplorerView() {
  const state = useSyncExternalStore(explorerNavigator.subscribe, explorerNavigator.getSnapshot);
  const directory = state.directory;
  const isLoading = state.status === "loading";
  const canGoBack = !isLoading && state.historyIndex > 0;
  const canGoForward = !isLoading && state.historyIndex < state.history.length - 1;
  const canGoUp = !isLoading && (directory?.breadcrumbs.length ?? 0) > 1;

  useEffect(() => {
    if (explorerNavigator.getSnapshot().status === "idle") {
      void explorerNavigator.initialize();
    }
  }, []);

  const retry = () => {
    if (directory) {
      void explorerNavigator.navigate(directory.path);
      return;
    }

    void explorerNavigator.initialize();
  };

  return (
    <main className="h-full bg-background">
      <section className="flex h-full w-full flex-col overflow-hidden">
        <header className="flex h-14 shrink-0 items-center gap-1 border-b px-2 sm:px-3">
          <div className="flex shrink-0 items-center gap-0.5">
            <Button
              aria-label="后退"
              disabled={!canGoBack}
              onClick={() => void explorerNavigator.goBack()}
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
              onClick={() => void explorerNavigator.goForward()}
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
              onClick={() => void explorerNavigator.goUp()}
              size="icon-sm"
              title="上一级"
              type="button"
              variant="ghost"
            >
              <ArrowUpIcon />
            </Button>
          </div>

          <div className="min-w-0 flex-1 px-2">
            {directory ? (
              <ExplorerBreadcrumbs
                breadcrumbs={directory.breadcrumbs}
                onNavigate={(breadcrumb) => void explorerNavigator.navigateBreadcrumb(breadcrumb)}
              />
            ) : (
              <Skeleton className="h-4 w-48 max-w-full" />
            )}
          </div>

          <Button
            aria-label="刷新"
            disabled={isLoading || !directory}
            onClick={() => directory && void explorerNavigator.navigate(directory.path)}
            size="icon-sm"
            title="刷新"
            type="button"
            variant="ghost"
          >
            <RefreshCwIcon className={cn(isLoading && "animate-spin")} />
          </Button>
        </header>

        {state.error && directory && (
          <div className="shrink-0 p-3 pb-0">
            <ExplorerErrorAlert message={state.error.message} onRetry={retry} />
          </div>
        )}

        {directory ? (
          <FileList
            entries={directory.entries}
            isLoading={isLoading}
            onOpenDirectory={(path) => void explorerNavigator.navigate(path)}
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
