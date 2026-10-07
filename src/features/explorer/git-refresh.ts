import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { events } from "@/bindings";

import { GIT_BRANCHES_QUERY_KEY } from "./git-branches";
import { GIT_REFRESH_DELAY_MS, GIT_STATUS_QUERY_KEY } from "./git-status";

/**
 * 目录变更时刷新 Git 查询，**每个窗口订阅一次**。
 *
 * 这段逻辑原先分别长在 `useGitStatus` 和 `useGitBranches` 里，于是每个挂载的
 * explorer 面都装两个监听器，各自去失效同一组全局 key：n 个面就是 2n 个监听器，
 * 一次目录变更触发 n 次同名的失效与重取。失效的作用域本来就是整个工作区（key 只到
 * `["git-status"]`，不带路径），所以一个订阅就够。
 *
 * 宿主是窗口级组件，不是每个面 —— 多订阅几份并不会给出更新的数据，只会让同一份
 * 工作重复做。
 */
export function useGitRefreshOnDirectoryChange(): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    let refreshTimeout: number | undefined;

    const unlistenPromise = events.explorerDirectoryChanged.listen(() => {
      window.clearTimeout(refreshTimeout);
      refreshTimeout = window.setTimeout(() => {
        refreshTimeout = undefined;
        // 强制重取，不受 `staleTime` 约束：这是真实的变化，不是重新聚焦。
        void queryClient.invalidateQueries({ queryKey: [GIT_STATUS_QUERY_KEY] });
        void queryClient.invalidateQueries({ queryKey: [GIT_BRANCHES_QUERY_KEY] });
      }, GIT_REFRESH_DELAY_MS);
    });

    return () => {
      window.clearTimeout(refreshTimeout);
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [queryClient]);
}
